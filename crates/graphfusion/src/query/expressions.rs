use crate::{gql as ast, Error, Result, SessionState, Value};
use datafusion::{
    arrow::datatypes::DataType,
    common::{Column, DFSchema, ScalarValue},
    logical_expr::{expr::BinaryExpr, Expr, ExprSchemable, Operator},
};

pub(super) struct Binder<'a> {
    session: &'a SessionState,
    schema: &'a DFSchema,
    bindings: Option<&'a super::graph::Bindings>,
    aggregates: bool,
    aliases: Option<&'a super::casefold::CaseFoldMap<Expr>>,
}

impl<'a> Binder<'a> {
    pub fn new(session: &'a SessionState, schema: &'a DFSchema) -> Self {
        Self {
            session,
            schema,
            bindings: None,
            aggregates: false,
            aliases: None,
        }
    }
    pub fn with_bindings(
        session: &'a SessionState,
        schema: &'a DFSchema,
        bindings: &'a super::graph::Bindings,
    ) -> Self {
        Self {
            session,
            schema,
            bindings: Some(bindings),
            aggregates: false,
            aliases: None,
        }
    }
    pub fn aggregating(mut self) -> Self {
        self.aggregates = true;
        self
    }
    pub fn aliases(mut self, aliases: &'a super::casefold::CaseFoldMap<Expr>) -> Self {
        self.aliases = Some(aliases);
        self
    }
    fn element(&self, name: &str) -> Result<&super::graph::ElementBinding> {
        self.bindings
            .ok_or_else(|| Error::InvalidQuery(format!("unbound element variable {name}")))?
            .element(name)
    }
    fn reference(&self, name: &str) -> Result<Expr> {
        let value = self.bind(&ast::Expr::Identifier(ast::Identifier::new(name)))?;
        match value.get_type(self.schema)? {
            DataType::Null => Ok(datafusion::logical_expr::cast(
                value,
                super::path_values::data_type(),
            )),
            ty if ty == super::path_values::data_type() => Ok(value),
            _ => Err(Error::InvalidQuery(format!(
                "{name} is not an element reference"
            ))),
        }
    }
    fn null_reference_predicate(&self, name: &str) -> Result<Expr> {
        let value = self.bind(&ast::Expr::Identifier(ast::Identifier::new(name)))?;
        if value.get_type(self.schema)? != DataType::Null {
            return Err(Error::InvalidQuery(format!(
                "{name} is not a resolved element reference"
            )));
        }
        Ok(datafusion::logical_expr::lit(ScalarValue::Boolean(None)))
    }
    pub fn equals(&self, left: Expr, right: Expr) -> Result<Expr> {
        let a = left.get_type(self.schema)?;
        let b = right.get_type(self.schema)?;
        if a != b && a != DataType::Null && b != DataType::Null && !(numeric(&a) && numeric(&b)) {
            return Err(Error::InvalidQuery(format!(
                "incomparable types {a} and {b}"
            )));
        }
        Ok(super::arithmetic::exact_numeric_compare(
            left.clone(),
            Operator::Eq,
            right.clone(),
            &a,
            &b,
        )
        .unwrap_or_else(|| left.eq(right)))
    }

    pub fn predicate(&self, expr: &ast::Expr) -> Result<Expr> {
        let expr = self.bind(expr)?;
        self.require(&expr, boolean, "boolean predicate")?;
        Ok(expr)
    }

    fn require(&self, expr: &Expr, accepts: fn(&DataType) -> bool, expected: &str) -> Result<()> {
        let data_type = expr.get_type(self.schema)?;
        if accepts(&data_type) {
            Ok(())
        } else {
            Err(Error::InvalidQuery(format!(
                "expected {expected}, got {data_type}"
            )))
        }
    }

    pub fn bind(&self, expr: &ast::Expr) -> Result<Expr> {
        use ast::Expr as E;
        Ok(match expr {
            E::Literal(value) => datafusion::logical_expr::lit(match value {
                ast::Literal::Null => ScalarValue::Null,
                ast::Literal::Unknown => ScalarValue::Boolean(None),
                ast::Literal::Boolean(v) => ScalarValue::Boolean(Some(*v)),
                ast::Literal::Integer(v) => ScalarValue::Int64(Some(*v)),
                ast::Literal::Decimal(v) => {
                    if !v.is_finite() {
                        return Err(Error::InvalidQuery("non-finite decimal literal".into()));
                    }
                    ScalarValue::Float64(Some(*v))
                }
                ast::Literal::String(v) => ScalarValue::Utf8(Some(v.clone())),
                ast::Literal::Bytes(v) => ScalarValue::Binary(Some(v.clone())),
                _ => return Err(super::unsupported("temporal query literals")),
            }),
            E::Parameter(name) => {
                let parameter = self
                    .session
                    .parameter(name)
                    .ok_or_else(|| Error::NotFound(format!("parameter {name}")))?;
                let mut expression = match &parameter.value {
                    Value::Null => datafusion::logical_expr::lit(ScalarValue::Null),
                    Value::Boolean(v) => datafusion::logical_expr::lit(*v),
                    Value::Integer(v) => datafusion::logical_expr::lit(*v),
                    Value::Float(v) => datafusion::logical_expr::lit(*v),
                    Value::String(v) => datafusion::logical_expr::lit(v.clone()),
                    Value::Bytes(v) => {
                        datafusion::logical_expr::lit(ScalarValue::Binary(Some(v.clone())))
                    }
                    Value::List(values) => list_parameter(values, self)?,
                    _ => return Err(super::unsupported("query parameter value type")),
                };
                if let Some(declared) = &parameter.declared_type {
                    let data_type = parameter_type(declared, &expression.get_type(self.schema)?)?;
                    expression = datafusion::logical_expr::cast(expression, data_type);
                }
                expression
            }

            E::Identifier(name) => {
                if let Some(value) = self.aliases.and_then(|aliases| aliases.get(&name.value)) {
                    return Ok(value.clone());
                }
                if let Some(bindings) = self.bindings {
                    if let Some(column) = bindings.scalars.get(&name.value) {
                        return Ok(Expr::Column(column.clone()));
                    }
                    if let Some(element) = bindings.elements.get(&name.value) {
                        return Ok(super::path_values::reference(
                            datafusion::logical_expr::lit(element.graph),
                            if element.kind == super::graph::ElementKind::Node {
                                "n"
                            } else {
                                "e"
                            },
                            element.ensure_live(element.column(crate::graph::ID), DataType::UInt64),
                        ));
                    }
                    return Err(Error::InvalidQuery(format!(
                        "unbound variable {}",
                        name.value
                    )));
                }
                if !self.schema.has_column_with_unqualified_name(&name.value) {
                    return Err(Error::InvalidQuery(format!(
                        "unbound variable {}",
                        name.value
                    )));
                }
                // Do not parse dots or quotes in an already-decoded GQL identifier as SQL.
                Expr::Column(Column::new_unqualified(&name.value))
            }
            E::Property { base, key } => {
                let E::Identifier(name) = base.as_ref() else {
                    return Err(super::unsupported("nested property access"));
                };
                if let Some(element) = self.bindings.and_then(|b| b.elements.get(&name.value)) {
                    element.property(&key.value)
                } else if let Some(reference) =
                    self.bindings.and_then(|b| b.references.get(&name.value))
                {
                    reference.property(self.reference(&name.value)?, &key.value, self.schema)?
                } else if self.bind(base)?.get_type(self.schema)? == DataType::Null {
                    datafusion::logical_expr::lit(ScalarValue::Null)
                } else {
                    return Err(Error::InvalidQuery(format!(
                        "{} is not a resolved element reference",
                        name.value
                    )));
                }
            }
            E::ElementId { variable } => {
                if self
                    .aliases
                    .is_some_and(|aliases| aliases.contains_key(&variable.value))
                {
                    super::path_values::identity(self.reference(&variable.value)?, self.schema)?
                } else if let Some(column) =
                    self.bindings.and_then(|b| b.scalars.get(&variable.value))
                {
                    super::path_values::identity(Expr::Column(column.clone()), self.schema)?
                } else {
                    self.element(&variable.value)?.identity()
                }
            }
            E::PropertyExists { variable, property } => {
                if let Some(element) = self.bindings.and_then(|b| b.elements.get(&variable.value)) {
                    element.property_exists(&property.value)
                } else if let Some(reference) = self
                    .bindings
                    .and_then(|b| b.references.get(&variable.value))
                {
                    reference.property_exists(self.reference(&variable.value)?, &property.value)
                } else {
                    self.null_reference_predicate(&variable.value)?
                }
            }
            E::IsLabeled {
                variable,
                negated,
                label_expression,
            } => {
                let expression = if let Some(element) =
                    self.bindings.and_then(|b| b.elements.get(&variable.value))
                {
                    element.label(label_expression)
                } else if let Some(reference) = self
                    .bindings
                    .and_then(|b| b.references.get(&variable.value))
                {
                    reference.label(self.reference(&variable.value)?, label_expression)
                } else {
                    self.null_reference_predicate(&variable.value)?
                };
                if *negated {
                    Expr::Not(Box::new(expression))
                } else {
                    expression
                }
            }
            E::IsDirected { variable, negated } => {
                let expression = if let Some(element) =
                    self.bindings.and_then(|b| b.elements.get(&variable.value))
                {
                    if element.kind != super::graph::ElementKind::Edge {
                        return Err(Error::InvalidQuery("IS DIRECTED requires an edge".into()));
                    }
                    element.ensure_live(element.column("__gf_directed"), DataType::Boolean)
                } else if let Some(reference) = self
                    .bindings
                    .and_then(|b| b.references.get(&variable.value))
                {
                    reference.directed(self.reference(&variable.value)?)
                } else {
                    self.null_reference_predicate(&variable.value)?
                };
                if *negated {
                    Expr::Not(Box::new(expression))
                } else {
                    expression
                }
            }
            E::Same { variables } | E::AllDifferent { variables } => {
                let elements = variables
                    .iter()
                    .map(|name| self.reference(&name.value))
                    .collect::<Result<Vec<_>>>()?;
                let mut result = datafusion::logical_expr::lit(true);
                for (i, left) in elements.iter().enumerate() {
                    for right in &elements[i + 1..] {
                        let same = left.clone().eq(right.clone());
                        result = result.and(if matches!(expr, E::AllDifferent { .. }) {
                            Expr::Not(Box::new(same))
                        } else {
                            same
                        });
                    }
                }
                result
            }
            E::Unary { op, expr } => {
                let expr = self.bind(expr)?;
                match op {
                    ast::UnaryOp::Not => {
                        self.require(&expr, boolean, "boolean operand")?;
                        Expr::Not(Box::new(expr))
                    }
                    ast::UnaryOp::Pos | ast::UnaryOp::Neg => {
                        self.require(&expr, numeric, "numeric operand")?;
                        if *op == ast::UnaryOp::Neg {
                            let data_type = if expr.get_type(self.schema)? == DataType::Float64 {
                                DataType::Float64
                            } else {
                                DataType::Int64
                            };
                            super::arithmetic::checked(
                                datafusion::logical_expr::lit(0_i64),
                                Operator::Minus,
                                expr,
                                data_type,
                            )
                        } else {
                            expr
                        }
                    }
                }
            }
            E::Binary { left, op, right } => {
                let mut left = self.bind(left)?;
                let mut right = self.bind(right)?;
                use ast::BinaryOp as B;
                let operator = match op {
                    B::And | B::Or | B::Xor => {
                        self.require(&left, boolean, "boolean operand")?;
                        self.require(&right, boolean, "boolean operand")?;
                        return Ok(match op {
                            B::And => logical_and(left, right),
                            B::Or => logical_or(left, right),
                            _ => logical_xor(left, right),
                        });
                    }
                    B::Add | B::Sub | B::Mul | B::Div => {
                        self.require(&left, numeric, "numeric operand")?;
                        self.require(&right, numeric, "numeric operand")?;
                        match op {
                            B::Add => Operator::Plus,
                            B::Sub => Operator::Minus,
                            B::Mul => Operator::Multiply,
                            _ => Operator::Divide,
                        }
                    }
                    B::Concat => {
                        self.require(&left, string, "string operand")?;
                        self.require(&right, string, "string operand")?;
                        // Supply the string context even when neither operand has a type.
                        if left.get_type(self.schema)? == DataType::Null {
                            left = left.cast_to(&DataType::Utf8, self.schema)?;
                        }
                        if right.get_type(self.schema)? == DataType::Null {
                            right = right.cast_to(&DataType::Utf8, self.schema)?;
                        }
                        Operator::StringConcat
                    }
                    B::Eq | B::Neq | B::Lt | B::Le | B::Gt | B::Ge => {
                        let a = left.get_type(self.schema)?;
                        let b = right.get_type(self.schema)?;
                        if is_list(&a) || is_list(&b) {
                            // Empty lists are List(Null) and numeric lists keep their
                            // element type. Widen to one list type before comparing.
                            let unified = common_value_type(&a, &b).map_err(|_| {
                                Error::InvalidQuery(format!("incomparable types {a} and {b}"))
                            })?;
                            if a != unified {
                                left = left.cast_to(&unified, self.schema)?;
                            }
                            if b != unified {
                                right = right.cast_to(&unified, self.schema)?;
                            }
                        } else if a != b
                            && a != DataType::Null
                            && b != DataType::Null
                            && !(numeric(&a) && numeric(&b))
                        {
                            return Err(Error::InvalidQuery(format!(
                                "incomparable types {a} and {b}"
                            )));
                        }
                        let operator = match op {
                            B::Eq => Operator::Eq,
                            B::Neq => Operator::NotEq,
                            B::Lt => Operator::Lt,
                            B::Le => Operator::LtEq,
                            B::Gt => Operator::Gt,
                            _ => Operator::GtEq,
                        };
                        if let Some(expr) = super::arithmetic::exact_numeric_compare(
                            left.clone(),
                            operator,
                            right.clone(),
                            &a,
                            &b,
                        ) {
                            return Ok(expr);
                        }
                        operator
                    }
                };
                if matches!(op, B::Add | B::Sub | B::Mul | B::Div) {
                    let left_type = left.get_type(self.schema)?;
                    let right_type = right.get_type(self.schema)?;
                    if matches!(
                        (&left_type, &right_type),
                        (DataType::Int64, DataType::Float64) | (DataType::Float64, DataType::Int64)
                    ) {
                        return Ok(super::arithmetic::checked_mixed(
                            left,
                            operator,
                            right,
                            left_type == DataType::Int64,
                        ));
                    }
                    let data_type =
                        if left_type == DataType::Float64 || right_type == DataType::Float64 {
                            DataType::Float64
                        } else {
                            DataType::Int64
                        };
                    super::arithmetic::checked(left, operator, right, data_type)
                } else if matches!(op, B::Eq | B::Neq | B::Lt | B::Le | B::Gt | B::Ge) {
                    let left_type = left.get_type(self.schema)?;
                    let right_type = right.get_type(self.schema)?;
                    if super::lists::is_list(&left_type) && super::lists::is_list(&right_type) {
                        // DataFusion treats null list elements as equal and a null
                        // element beside a value as unequal. GQL comparisons stay unknown.
                        super::lists::compare(left, right, &left_type, &right_type, operator)?
                    } else {
                        Expr::BinaryExpr(BinaryExpr::new(Box::new(left), operator, Box::new(right)))
                    }
                } else {
                    Expr::BinaryExpr(BinaryExpr::new(Box::new(left), operator, Box::new(right)))
                }
            }
            E::IsNull { expr, negated } => {
                let expr = if let E::Identifier(name) = expr.as_ref() {
                    if let Some(binding) = self
                        .bindings
                        .filter(|_| !self.aliases.is_some_and(|a| a.contains_key(&name.value)))
                        .and_then(|b| b.elements.get(&name.value))
                    {
                        binding.column(crate::graph::ID)
                    } else {
                        self.bind(expr)?
                    }
                } else {
                    self.bind(expr)?
                };
                if *negated {
                    expr.is_not_null()
                } else {
                    expr.is_null()
                }
            }
            E::IsUnknown { expr, negated } => {
                let expr = self.bind(expr)?;
                self.require(&expr, boolean, "boolean operand")?;
                if *negated {
                    expr.is_not_null()
                } else {
                    expr.is_null()
                }
            }
            E::IsTruth {
                expr,
                negated,
                value,
            } => {
                let expr = self.bind(expr)?;
                self.require(&expr, boolean, "boolean operand")?;
                // `IS TRUE` is planned as a non-nullable boolean, but DataFusion
                // executes it as a nullable distinctness comparison. Build that
                // comparison here so DISTINCT sees one nullability.
                let op = if *negated {
                    Operator::IsDistinctFrom
                } else {
                    Operator::IsNotDistinctFrom
                };
                Expr::BinaryExpr(BinaryExpr::new(
                    Box::new(expr),
                    op,
                    Box::new(datafusion::logical_expr::lit(*value)),
                ))
            }
            E::Coalesce(args) => {
                let args = coalesce_arguments(args)
                    .into_iter()
                    .map(|arg| self.bind(arg))
                    .collect::<Result<Vec<_>>>()?;
                self.compatible(&args)?;
                let args = self.align_lists(args)?;
                self.coalesce(args)?
            }
            E::NullIf { left, right } => {
                let left = self.bind(left)?;
                let right = self.bind(right)?;
                self.compatible(&[left.clone(), right.clone()])?;
                let (left, right) = self.align_list_pair(left, right)?;
                let left_type = left.get_type(self.schema)?;
                let right_type = right.get_type(self.schema)?;
                if left_type == DataType::Null && right_type == DataType::Null {
                    // Both values are null; avoid DataFusion's fallback to a string type.
                    left
                } else if super::lists::is_list(&left_type) && super::lists::is_list(&right_type) {
                    // NULLIF is CASE WHEN equal THEN NULL ELSE left. Unknown equality
                    // keeps the left list; only a true comparison removes it.
                    let equal = super::lists::compare(
                        left.clone(),
                        right,
                        &left_type,
                        &right_type,
                        Operator::Eq,
                    )?;
                    let null = datafusion::logical_expr::lit(ScalarValue::try_from(&left_type)?);
                    Expr::Case(datafusion::logical_expr::expr::Case::new(
                        None,
                        vec![(Box::new(equal), Box::new(null))],
                        Some(Box::new(left)),
                    ))
                } else {
                    super::arithmetic::exact_nullif(
                        left.clone(),
                        right.clone(),
                        &left_type,
                        &right_type,
                    )
                    .unwrap_or_else(|| datafusion::functions::core::expr_fn::nullif(left, right))
                }
            }
            E::NumericFunction {
                function: ast::NumericFunctionKind::PathLength,
                args,
            } => {
                if args.len() != 1 {
                    return Err(Error::InvalidQuery(
                        "PATH_LENGTH requires one argument".into(),
                    ));
                }
                super::paths::length(self.bind(&args[0])?, self.schema)?
            }
            E::Elements { path } => super::paths::elements(self.bind(path)?, self.schema)?,
            E::List(values) => {
                let values = values
                    .iter()
                    .map(|e| self.bind(e))
                    .collect::<Result<Vec<_>>>()?;
                self.array(values)?
            }
            E::Function {
                name,
                quantifier,
                args,
            } if self.aggregates => {
                if matches!(name.value.to_ascii_uppercase().as_str(), "MIN" | "MAX") {
                    if let Some(bindings) = self.bindings {
                        for arg in args {
                            ensure_one_family(bindings, arg, &name.value)?;
                        }
                    }
                }
                if *quantifier == Some(ast::SetQuantifier::Distinct) {
                    if let Some(bindings) = self.bindings {
                        for arg in args {
                            ensure_one_family(bindings, arg, "DISTINCT")?;
                        }
                    }
                }
                let star = args.len() == 1 && matches!(args[0], E::Wildcard);
                let bound = if star {
                    vec![]
                } else {
                    args.iter()
                        .map(|arg| {
                            if name.value.eq_ignore_ascii_case("COUNT") {
                                if let E::Identifier(name) = arg {
                                    if let Some(binding) = self
                                        .bindings
                                        .filter(|_| {
                                            !self
                                                .aliases
                                                .is_some_and(|a| a.contains_key(&name.value))
                                        })
                                        .and_then(|b| b.elements.get(&name.value))
                                    {
                                        return Ok(binding.identity());
                                    }
                                }
                            }
                            self.bind(arg)
                        })
                        .collect::<Result<Vec<_>>>()?
                };
                super::aggregates::call(&name.value, *quantifier, bound, star, self.schema)?
            }
            _ => return Err(super::unsupported("query expression")),
        })
    }

    fn compatible(&self, expressions: &[Expr]) -> Result<()> {
        let mut previous = DataType::Null;
        for expr in expressions {
            previous = common_value_type(&previous, &expr.get_type(self.schema)?)?;
        }
        Ok(())
    }

    /// Build a list after finding one element type. An untyped null does not
    /// coerce into a byte string inside `make_array`, so cast it to the element
    /// type first. Other families already accept a null argument.
    fn array(&self, values: Vec<Expr>) -> Result<Expr> {
        let mut element = DataType::Null;
        for expr in &values {
            element = common_value_type(&element, &expr.get_type(self.schema)?)?;
        }
        let values = values
            .into_iter()
            .map(|expr| {
                if element != DataType::Null && expr.get_type(self.schema)? == DataType::Null {
                    Ok(expr.cast_to(&element, self.schema)?)
                } else {
                    Ok(expr)
                }
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(datafusion::functions_nested::expr_fn::make_array(values))
    }

    /// `coalesce` accepts an untyped null beside a number or string, but not
    /// beside a byte string. Cast those nulls to binary once the other
    /// arguments have established that family.
    fn coalesce(&self, args: Vec<Expr>) -> Result<Expr> {
        let mut element = DataType::Null;
        for expr in &args {
            element = common_value_type(&element, &expr.get_type(self.schema)?)?;
        }
        let args = args
            .into_iter()
            .map(|expr| {
                if element == DataType::Binary && expr.get_type(self.schema)? == DataType::Null {
                    Ok(expr.cast_to(&DataType::Binary, self.schema)?)
                } else {
                    Ok(expr)
                }
            })
            .collect::<Result<Vec<_>>>()?;
        let types = args
            .iter()
            .map(|arg| arg.get_type(self.schema))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let value = super::arithmetic::lazy_coalesce(args, &types, element)?;
        if value.column_refs().is_empty()
            && datafusion::logical_expr::utils::find_aggregate_exprs([&value]).is_empty()
        {
            use datafusion::optimizer::simplify_expressions::{ExprSimplifier, SimplifyContext};
            Ok(ExprSimplifier::new(SimplifyContext::default()).simplify(value)?)
        } else {
            Ok(value)
        }
    }

    fn align_list_pair(&self, left: Expr, right: Expr) -> Result<(Expr, Expr)> {
        let mut aligned = self.align_lists(vec![left, right])?;
        let right = aligned.pop().expect("aligned pair");
        let left = aligned.pop().expect("aligned pair");
        Ok((left, right))
    }

    /// Cast list operands to their common element type. Scalar families stay
    /// untouched so numeric widening remains DataFusion's coercion.
    fn align_lists(&self, expressions: Vec<Expr>) -> Result<Vec<Expr>> {
        let mut types = Vec::with_capacity(expressions.len());
        for expr in &expressions {
            types.push(expr.get_type(self.schema)?);
        }
        if !types.iter().any(is_list) {
            return Ok(expressions);
        }
        let mut unified = DataType::Null;
        for data_type in &types {
            unified = common_value_type(&unified, data_type)?;
        }
        let mut aligned = Vec::with_capacity(expressions.len());
        for (expr, data_type) in expressions.into_iter().zip(types) {
            aligned.push(if data_type == unified {
                expr
            } else {
                expr.cast_to(&unified, self.schema)?
            });
        }
        Ok(aligned)
    }
}

fn is_list(data_type: &DataType) -> bool {
    matches!(data_type, DataType::List(_) | DataType::LargeList(_))
}

pub(super) fn common_value_type(left: &DataType, right: &DataType) -> Result<DataType> {
    if left == right || *right == DataType::Null {
        return Ok(left.clone());
    }
    if *left == DataType::Null {
        return Ok(right.clone());
    }
    let incompatible =
        || Error::InvalidQuery(format!("incompatible value types {left} and {right}"));
    match (left, right) {
        (DataType::List(a), DataType::List(b)) => Ok(DataType::new_list(
            common_value_type(a.data_type(), b.data_type())?,
            a.is_nullable() || b.is_nullable(),
        )),
        (a, b) if numeric(a) && numeric(b) => {
            datafusion::logical_expr::binary::binary_numeric_coercion(a, b).ok_or_else(incompatible)
        }
        _ => Err(incompatible()),
    }
}

fn parameter_type(declared: &crate::types::ValueType, inferred: &DataType) -> Result<DataType> {
    Ok(match declared.kind.as_str() {
        "any" => inferred.clone(),
        "boolean" => DataType::Boolean,
        "integer" => DataType::Int64,
        // Decimal literals and parameters are f64; precision and scale were checked at declaration.
        "float" | "decimal" => DataType::Float64,
        "string" => DataType::Utf8,
        "bytes" => DataType::Binary,
        "list" | "array" => {
            let inferred = match inferred {
                DataType::List(field) => field.data_type(),
                _ => &DataType::Null,
            };
            let element = declared
                .arguments
                .first()
                .ok_or_else(|| Error::InvalidQuery("list parameter has no element type".into()))?;
            DataType::new_list(parameter_type(element, inferred)?, true)
        }
        _ => return Err(super::unsupported("declared query parameter type")),
    })
}

fn list_parameter(values: &[Value], binder: &Binder<'_>) -> Result<Expr> {
    use datafusion::logical_expr::lit;
    let args = values
        .iter()
        .map(|value| {
            Ok(match value {
                Value::Null => lit(ScalarValue::Null),
                Value::Boolean(v) => lit(*v),
                Value::Integer(v) => lit(*v),
                Value::Float(v) => lit(*v),
                Value::String(v) => lit(v.clone()),
                Value::Bytes(v) => lit(ScalarValue::Binary(Some(v.clone()))),
                Value::List(v) => list_parameter(v, binder)?,
                _ => return Err(super::unsupported("list parameter element type")),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    binder.array(args)
}

fn boolean(data_type: &DataType) -> bool {
    matches!(data_type, DataType::Boolean | DataType::Null)
}
fn numeric(data_type: &DataType) -> bool {
    data_type.is_numeric() || *data_type == DataType::Null
}
fn string(data_type: &DataType) -> bool {
    matches!(
        data_type,
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View | DataType::Null
    )
}

// Flatten the associative abbreviation before binding when a caller only has
// a schema. Working-table planning additionally shares arguments through columns.
fn coalesce_arguments(args: &[ast::Expr]) -> Vec<&ast::Expr> {
    fn collect<'a>(expr: &'a ast::Expr, result: &mut Vec<&'a ast::Expr>) {
        if let ast::Expr::Coalesce(args) = expr {
            for arg in args {
                collect(arg, result);
            }
        } else {
            result.push(expr);
        }
    }
    let mut result = Vec::new();
    for arg in args {
        collect(arg, &mut result);
    }
    result
}

/// Decode nullable integer truth flags without exposing Boolean COALESCE to
/// optimizer rewrites that can generate incorrect dynamic join filters.
pub(super) fn boolean_value(flag: Expr) -> Expr {
    use datafusion::{
        arrow::array::{BooleanArray, Int64Array},
        error::DataFusionError,
        logical_expr::{create_udf, ColumnarValue, ScalarUDF, Volatility},
    };
    use std::sync::{Arc, OnceLock};
    static FUNCTION: OnceLock<ScalarUDF> = OnceLock::new();
    FUNCTION
        .get_or_init(|| {
            create_udf(
                "gql_boolean_value",
                vec![DataType::Int64],
                DataType::Boolean,
                Volatility::Immutable,
                Arc::new(|args| {
                    let arrays = ColumnarValue::values_to_arrays(args)?;
                    let flags =
                        arrays[0]
                            .as_any()
                            .downcast_ref::<Int64Array>()
                            .ok_or_else(|| {
                                DataFusionError::Internal("expected integer truth flags".into())
                            })?;
                    let output: BooleanArray =
                        flags.iter().map(|value| value.map(|n| n != 0)).collect();
                    if args
                        .iter()
                        .all(|arg| matches!(arg, ColumnarValue::Scalar(_)))
                    {
                        Ok(ColumnarValue::Scalar(ScalarValue::try_from_array(
                            &output, 0,
                        )?))
                    } else {
                        Ok(ColumnarValue::Array(Arc::new(output)))
                    }
                }),
            )
        })
        .call(vec![flag])
}

/// A value-preserving wrapper retains the stored families of its properties.
pub(super) fn stored_property_types(
    bindings: Option<&super::graph::Bindings>,
    expr: &ast::Expr,
) -> Option<Vec<DataType>> {
    if let ast::Expr::Identifier(name) = expr {
        return bindings?.value_types.get(&name.value).cloned();
    }
    let list_args = match expr {
        ast::Expr::List(args) => Some(args.as_slice()),
        ast::Expr::Function { name, args, .. }
            if name.value.eq_ignore_ascii_case("COLLECT_LIST") =>
        {
            Some(args.as_slice())
        }
        _ => None,
    };
    if let Some(args) = list_args {
        let mut result = Vec::new();
        let mut known = false;
        for arg in args {
            if let Some(types) = stored_property_types(bindings, arg) {
                known = true;
                for ty in types {
                    let list = DataType::new_list(ty, true);
                    if !result.contains(&list) {
                        result.push(list);
                    }
                }
            }
        }
        return known.then_some(result);
    }
    if let ast::Expr::Coalesce(args) = expr {
        let mut result = Vec::new();
        let mut known = false;
        for arg in args {
            if let Some(types) = stored_property_types(bindings, arg) {
                known = true;
                for ty in types {
                    if !result.contains(&ty) {
                        result.push(ty);
                    }
                }
            }
            if known_non_null(bindings, arg) {
                break;
            }
        }
        return known.then_some(result);
    }
    if let ast::Expr::NullIf { left, .. } = expr {
        return stored_property_types(bindings, left);
    }
    let ast::Expr::Property { base, key } = expr else {
        return None;
    };
    let ast::Expr::Identifier(name) = base.as_ref() else {
        return None;
    };
    let bindings = bindings?;
    let sources = if let Some(element) = bindings.elements.get(&name.value) {
        vec![element.clone()]
    } else if let Some(reference) = bindings.references.get(&name.value) {
        reference.parts.clone()
    } else {
        bindings.reference_sources.get(&name.value)?.clone()
    };
    let mut result = Vec::new();
    for source in sources {
        let Some(graph) = bindings.sources.get(&source.graph) else {
            continue;
        };
        let tables: Vec<&crate::graph::Table> = match source.kind {
            super::graph::ElementKind::Node => graph.nodes.iter().map(|table| &table.0).collect(),
            super::graph::ElementKind::Edge => {
                graph.edges.iter().map(|table| &table.table).collect()
            }
        };
        for table in tables {
            if !source.source_labels.contains(&table.labels) {
                continue;
            }
            if let Some(field) = crate::graph::property_field(&table.schema, &key.value) {
                let ty = field.data_type();
                if *ty != DataType::Null && !result.contains(ty) {
                    result.push(ty.clone());
                }
            }
        }
    }
    Some(result)
}

pub(super) fn ensure_one_family(
    bindings: &super::graph::Bindings,
    expr: &ast::Expr,
    operation: &str,
) -> Result<()> {
    if value_types_deferred(bindings, expr) {
        return Ok(());
    }
    let Some(types) = stored_property_types(Some(bindings), expr) else {
        return Ok(());
    };
    let mut families = Vec::new();
    for ty in &types {
        let Some(family) = value_family(ty) else {
            continue;
        };
        if !families.contains(&family) {
            families.push(family);
        }
    }
    if families.len() <= 1 {
        return Ok(());
    }
    let mut labels = types
        .iter()
        .filter(|ty| value_family(ty).is_some())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    labels.sort();
    Err(Error::InvalidQuery(format!(
        "{} cannot combine {}",
        operation.to_ascii_uppercase(),
        labels.join(" and ")
    )))
}

fn value_family(ty: &DataType) -> Option<String> {
    Some(match ty {
        DataType::Null => return None,
        ty if ty.is_numeric() => "numeric".into(),
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => "string".into(),
        DataType::Binary | DataType::LargeBinary | DataType::BinaryView => "bytes".into(),
        DataType::List(field) | DataType::LargeList(field) | DataType::FixedSizeList(field, _) => {
            format!("list<{}>", value_family(field.data_type())?)
        }
        other => other.to_string(),
    })
}

/// During OTHERWISE validation a value can still come from different branches.
/// Its family constraint is checked again once execution selects that branch.
#[derive(Clone, Default)]
pub(super) struct StoredTypes {
    values: super::casefold::CaseFoldMap<Vec<DataType>>,
    deferred: super::casefold::CaseFoldMap<()>,
    non_null: super::casefold::CaseFoldMap<()>,
    schema: Option<datafusion::common::DFSchemaRef>,
}

impl std::ops::Deref for StoredTypes {
    type Target = super::casefold::CaseFoldMap<Vec<DataType>>;
    fn deref(&self) -> &Self::Target {
        &self.values
    }
}

impl StoredTypes {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn insert(&mut self, name: impl AsRef<str>, types: Vec<DataType>) -> Option<Vec<DataType>> {
        self.deferred.remove(name.as_ref());
        self.non_null.remove(name.as_ref());
        self.values.insert(name, types)
    }
    pub fn extend(&mut self, other: Self) {
        for (name, types) in other.values {
            self.insert(&name, types);
            self.mark_deferred(&name, other.deferred.contains_key(&name));
            self.mark_non_null(&name, other.non_null.contains_key(&name));
        }
    }
    pub fn clear(&mut self) {
        self.values.clear();
        self.deferred.clear();
        self.non_null.clear();
        self.schema = None;
    }
    pub fn is_deferred(&self, name: &str) -> bool {
        self.deferred.contains_key(name)
    }
    pub fn mark_deferred(&mut self, name: &str, deferred: bool) {
        if deferred {
            self.deferred.insert(name, ());
        } else {
            self.deferred.remove(name);
        }
    }
    pub fn set_schema(&mut self, schema: datafusion::common::DFSchemaRef) {
        self.schema = Some(schema);
    }
    pub fn is_non_null(&self, name: &str) -> bool {
        self.non_null.contains_key(name)
    }
    pub fn mark_non_null(&mut self, name: &str, non_null: bool) {
        if non_null {
            self.non_null.insert(name, ());
        } else {
            self.non_null.remove(name);
        }
    }
    pub fn defer_all(&mut self) {
        for name in self.values.keys() {
            self.deferred.insert(name, ());
        }
    }
}

pub(super) fn value_types_deferred(scope: &super::graph::Bindings, expr: &ast::Expr) -> bool {
    if let ast::Expr::Identifier(name) = expr {
        return scope.value_types.is_deferred(&name.value);
    }
    if let ast::Expr::Coalesce(args) = expr {
        for arg in args {
            if value_types_deferred(scope, arg) {
                return true;
            }
            if known_non_null(Some(scope), arg) {
                break;
            }
        }
        return false;
    }
    super::native::children(expr)
        .into_iter()
        .any(|child| value_types_deferred(scope, child))
}

/// A proof about successful evaluations; expressions that can return null keep
/// their fallback provenance. Stored operators are not inferred from unified
/// input types, since incompatible fragments can produce null.
pub(super) fn known_non_null(bindings: Option<&super::graph::Bindings>, expr: &ast::Expr) -> bool {
    use ast::Expr as E;
    match expr {
        E::Literal(ast::Literal::Null | ast::Literal::Unknown) => false,
        E::Literal(_)
        | E::List(_)
        | E::TypedList { .. }
        | E::IsNull { .. }
        | E::IsUnknown { .. }
        | E::IsTruth { .. } => true,
        E::Coalesce(args) => args.iter().any(|arg| known_non_null(bindings, arg)),
        E::Function { name, .. } => {
            name.value.eq_ignore_ascii_case("COUNT")
                || name.value.eq_ignore_ascii_case("COLLECT_LIST")
        }
        E::Identifier(name) => {
            let Some(scope) = bindings else {
                return false;
            };
            if scope.value_types.is_non_null(&name.value) {
                return true;
            }
            let Some(schema) = &scope.value_types.schema else {
                return false;
            };
            let value = scope
                .scalars
                .get(&name.value)
                .cloned()
                .map(Expr::Column)
                .or_else(|| {
                    scope
                        .elements
                        .get(&name.value)
                        .map(|binding| binding.column(crate::graph::ID))
                });
            value.is_some_and(|value| value.nullable(schema).is_ok_and(|nullable| !nullable))
        }
        E::Property { base, key } => {
            let Some(scope) = bindings else {
                return false;
            };
            let E::Identifier(name) = base.as_ref() else {
                return false;
            };
            let Some(schema) = &scope.value_types.schema else {
                return false;
            };
            let value = if let Some(binding) = scope.elements.get(&name.value) {
                Some(binding.property(&key.value))
            } else if let (Some(reference), Some(column)) = (
                scope.references.get(&name.value),
                scope.scalars.get(&name.value),
            ) {
                reference
                    .property(Expr::Column(column.clone()), &key.value, schema)
                    .ok()
            } else {
                None
            };
            value.is_some_and(|value| value.nullable(schema).is_ok_and(|nullable| !nullable))
        }
        E::Unary { .. } | E::Binary { .. } if scalar_constant(expr) => {
            super::native::children(expr)
                .into_iter()
                .all(|child| known_non_null(bindings, child))
        }
        _ => false,
    }
}

fn scalar_constant(expr: &ast::Expr) -> bool {
    match expr {
        ast::Expr::Literal(_) => true,
        ast::Expr::Unary { .. } | ast::Expr::Binary { .. } => super::native::children(expr)
            .into_iter()
            .all(scalar_constant),
        _ => false,
    }
}

pub(super) fn merge_value_types(left: &mut StoredTypes, right: &StoredTypes) {
    for (name, types) in right.iter() {
        let mut values = left.get(name).cloned().unwrap_or_default();
        for ty in types {
            if !values.contains(ty) {
                values.push(ty.clone());
            }
        }
        let deferred = left.is_deferred(name) || right.is_deferred(name);
        let non_null = left.is_non_null(name) && right.is_non_null(name);
        left.insert(name.clone(), values);
        left.mark_deferred(name, deferred);
        left.mark_non_null(name, non_null);
    }
}

pub(super) fn result_value_types(
    body: &ast::QueryBody,
    scope: &super::graph::Bindings,
    schema: &DFSchema,
) -> StoredTypes {
    let mut sources = super::casefold::CaseFoldMap::new();
    for (i, item) in body.result_clause.items.iter().enumerate() {
        if matches!(item.expr, ast::Expr::Wildcard) {
            continue;
        }
        let name = item
            .alias
            .as_ref()
            .map(|name| name.value.clone())
            .unwrap_or_else(|| match &item.expr {
                ast::Expr::Identifier(name) => name.value.clone(),
                _ => format!("column_{}", i + 1),
            });
        sources.insert(name, &item.expr);
    }
    let mut result = StoredTypes::new();
    for field in schema.fields() {
        let types = if let Some(source) = sources.get(field.name()) {
            stored_property_types(Some(scope), source)
        } else {
            scope.value_types.get(field.name()).cloned()
        };
        result.insert(
            field.name().clone(),
            types.unwrap_or_else(|| vec![field.data_type().clone()]),
        );
        let deferred = sources
            .get(field.name())
            .map(|source| value_types_deferred(scope, source))
            .unwrap_or_else(|| scope.value_types.is_deferred(field.name()));
        result.mark_deferred(field.name(), deferred);
        let non_null = !field.is_nullable()
            || sources
                .get(field.name())
                .map(|source| known_non_null(Some(scope), source))
                .unwrap_or_else(|| scope.value_types.is_non_null(field.name()));
        result.mark_non_null(field.name(), non_null);
    }
    result
}

/// Integer CASE results keep boolean simplification from restoring eager batch
/// evaluation. Duplicate only the smaller operand, bounding both associations
/// of long chains while preserving left-to-right, per-row short circuiting.
pub(super) fn logical_and(left: Expr, right: Expr) -> Expr {
    logical_boolean(left, right, Operator::And)
}
pub(super) fn logical_or(left: Expr, right: Expr) -> Expr {
    logical_boolean(left, right, Operator::Or)
}
pub(super) fn logical_xor(left: Expr, right: Expr) -> Expr {
    logical_boolean(left, right, Operator::NotEq)
}
fn expression_size(expr: &Expr) -> usize {
    use datafusion::common::tree_node::{TreeNode, TreeNodeRecursion};
    let mut count = 0;
    expr.apply(|_| {
        count += 1;
        Ok(TreeNodeRecursion::Continue)
    })
    .expect("counting cannot fail");
    count
}
fn integer_case(operand: Option<Expr>, arms: Vec<(Expr, Expr)>, otherwise: Expr) -> Expr {
    Expr::Case(datafusion::logical_expr::expr::Case::new(
        operand.map(Box::new),
        arms.into_iter()
            .map(|(a, b)| (Box::new(a), Box::new(b)))
            .collect(),
        Some(Box::new(otherwise)),
    ))
}
fn logical_boolean(left: Expr, right: Expr, op: Operator) -> Expr {
    use datafusion::logical_expr::{cast, lit};
    let null = lit(ScalarValue::Int64(None));
    let flag = if expression_size(&left) <= expression_size(&right) {
        let (condition, decided) = match op {
            Operator::And => (
                Expr::BinaryExpr(BinaryExpr::new(
                    Box::new(left.clone()),
                    Operator::IsNotDistinctFrom,
                    Box::new(lit(false)),
                )),
                lit(0_i64),
            ),
            Operator::Or => (
                Expr::BinaryExpr(BinaryExpr::new(
                    Box::new(left.clone()),
                    Operator::IsNotDistinctFrom,
                    Box::new(lit(true)),
                )),
                lit(1_i64),
            ),
            Operator::NotEq => (left.clone().is_null(), null.clone()),
            _ => unreachable!(),
        };
        let remaining = Expr::BinaryExpr(BinaryExpr::new(Box::new(left), op, Box::new(right)));
        integer_case(
            None,
            vec![(condition, decided)],
            cast(remaining, DataType::Int64),
        )
    } else {
        let right = cast(right, DataType::Int64);
        let arms = match op {
            Operator::And => vec![(lit(0_i64), lit(0_i64)), (lit(1_i64), right.clone())],
            Operator::Or => vec![(lit(1_i64), lit(1_i64)), (lit(0_i64), right.clone())],
            Operator::NotEq => vec![
                (lit(0_i64), right.clone()),
                (
                    lit(1_i64),
                    datafusion::logical_expr::lit(1_i64) - right.clone(),
                ),
            ],
            _ => unreachable!(),
        };
        let otherwise = match op {
            Operator::And => integer_case(Some(right), vec![(lit(0_i64), lit(0_i64))], null),
            Operator::Or => integer_case(Some(right), vec![(lit(1_i64), lit(1_i64))], null),
            Operator::NotEq => null,
            _ => unreachable!(),
        };
        integer_case(Some(cast(left, DataType::Int64)), arms, otherwise)
    };
    boolean_value(flag)
}

#[cfg(test)]
mod boolean_plan_growth {
    use super::*;
    #[test]
    fn boolean_chains_have_bounded_expression_size() {
        for operator in [Operator::And, Operator::Or, Operator::NotEq] {
            for right_associated in [false, true] {
                let mut expr = Expr::Column(Column::new_unqualified("a"));
                for _ in 0..64 {
                    let next = Expr::Column(Column::new_unqualified("b"));
                    expr = if right_associated {
                        logical_boolean(next, expr, operator)
                    } else {
                        logical_boolean(expr, next, operator)
                    };
                }
                let count = expression_size(&expr);
                assert!(
                    count < 3000,
                    "{operator:?} right_associated={right_associated}: {count} nodes"
                );
            }
        }
    }
}
