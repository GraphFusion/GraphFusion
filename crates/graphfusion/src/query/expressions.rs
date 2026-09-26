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
    aliases: Option<&'a std::collections::BTreeMap<String, Expr>>,
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
    pub fn aliases(mut self, aliases: &'a std::collections::BTreeMap<String, Expr>) -> Self {
        self.aliases = Some(aliases);
        self
    }
    fn element(&self, name: &str) -> Result<&super::graph::ElementBinding> {
        self.bindings
            .ok_or_else(|| Error::InvalidQuery(format!("unbound element variable {name}")))?
            .element(name)
    }
    pub fn equals(&self, left: Expr, right: Expr) -> Result<Expr> {
        let a = left.get_type(self.schema)?;
        let b = right.get_type(self.schema)?;
        if a != b && a != DataType::Null && b != DataType::Null && !(numeric(&a) && numeric(&b)) {
            return Err(Error::InvalidQuery(format!(
                "incomparable types {a} and {b}"
            )));
        }
        Ok(left.eq(right))
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
                ast::Literal::Decimal(v) => ScalarValue::Float64(Some(*v)),
                ast::Literal::String(v) => ScalarValue::Utf8(Some(v.clone())),
                ast::Literal::Bytes(v) => ScalarValue::Binary(Some(v.clone())),
                _ => return Err(super::unsupported("temporal query literals")),
            }),
            E::Parameter(name) => {
                let parameter = self
                    .session
                    .parameters
                    .get(name)
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
                    if bindings.elements.contains_key(&name.value) {
                        return Err(super::unsupported(
                            "whole element values in results or scalar expressions",
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
                self.element(&name.value)?.property(&key.value)
            }
            E::ElementId { variable } => self.element(&variable.value)?.identity(),
            E::PropertyExists { variable, property } => self
                .element(&variable.value)?
                .property_exists(&property.value),
            E::IsLabeled {
                variable,
                negated,
                label_expression,
            } => {
                let expression = self.element(&variable.value)?.label(label_expression);
                if *negated {
                    Expr::Not(Box::new(expression))
                } else {
                    expression
                }
            }
            E::IsDirected { variable, negated } => {
                let element = self.element(&variable.value)?;
                if element.kind != super::graph::ElementKind::Edge {
                    return Err(Error::InvalidQuery("IS DIRECTED requires an edge".into()));
                }
                let expression = element.column("__gf_directed");
                if *negated {
                    Expr::Not(Box::new(expression))
                } else {
                    expression
                }
            }
            E::Same { variables } | E::AllDifferent { variables } => {
                let elements = variables
                    .iter()
                    .map(|name| self.element(&name.value))
                    .collect::<Result<Vec<_>>>()?;
                let mut result = datafusion::logical_expr::lit(true);
                for (i, left) in elements.iter().enumerate() {
                    for right in &elements[i + 1..] {
                        let same = if left.graph == right.graph && left.kind == right.kind {
                            left.column(crate::graph::ID)
                                .eq(right.column(crate::graph::ID))
                        } else {
                            left.nullable_predicate(
                                right.nullable_predicate(datafusion::logical_expr::lit(false)),
                            )
                        };
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
                        match op {
                            B::And => Operator::And,
                            B::Or => Operator::Or,
                            _ => Operator::NotEq,
                        }
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
                        if a != b
                            && a != DataType::Null
                            && b != DataType::Null
                            && !(numeric(&a) && numeric(&b))
                        {
                            return Err(Error::InvalidQuery(format!(
                                "incomparable types {a} and {b}"
                            )));
                        }
                        match op {
                            B::Eq => Operator::Eq,
                            B::Neq => Operator::NotEq,
                            B::Lt => Operator::Lt,
                            B::Le => Operator::LtEq,
                            B::Gt => Operator::Gt,
                            _ => Operator::GtEq,
                        }
                    }
                };
                if matches!(op, B::Add | B::Sub | B::Mul | B::Div) {
                    let data_type = if left.get_type(self.schema)? == DataType::Float64
                        || right.get_type(self.schema)? == DataType::Float64
                    {
                        DataType::Float64
                    } else {
                        DataType::Int64
                    };
                    super::arithmetic::checked(left, operator, right, data_type)
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
                let truth = if *value {
                    expr.is_true()
                } else {
                    expr.is_false()
                };
                if *negated {
                    Expr::Not(Box::new(truth))
                } else {
                    truth
                }
            }
            E::Coalesce(args) => {
                let args = args
                    .iter()
                    .map(|arg| self.bind(arg))
                    .collect::<Result<Vec<_>>>()?;
                self.compatible(&args)?;
                datafusion::functions::core::expr_fn::coalesce(args)
            }
            E::NullIf { left, right } => {
                let left = self.bind(left)?;
                let right = self.bind(right)?;
                self.compatible(&[left.clone(), right.clone()])?;
                if left.get_type(self.schema)? == DataType::Null
                    && right.get_type(self.schema)? == DataType::Null
                {
                    // Both values are null; avoid DataFusion's fallback to a string type.
                    left
                } else {
                    datafusion::functions::core::expr_fn::nullif(left, right)
                }
            }
            E::List(values) => {
                let values = values
                    .iter()
                    .map(|e| self.bind(e))
                    .collect::<Result<Vec<_>>>()?;
                self.compatible(&values)?;
                datafusion::functions_nested::expr_fn::make_array(values)
            }
            E::Function {
                name,
                quantifier,
                args,
            } if self.aggregates => {
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
}

fn common_value_type(left: &DataType, right: &DataType) -> Result<DataType> {
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
        "float" => DataType::Float64,
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
    binder.compatible(&args)?;
    Ok(datafusion::functions_nested::expr_fn::make_array(args))
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
