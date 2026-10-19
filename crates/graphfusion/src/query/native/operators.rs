//! Stored operators form a typed dataflow graph. Each node combines only its
//! immediate operands, and shared values are columns rather than expanded trees.
use super::super::expressions::common_value_type;
use super::*;

type Origins = BTreeMap<String, BTreeSet<String>>;
type Dependencies = BTreeSet<Column>;

#[derive(Clone)]
pub(super) struct Cached {
    pub expression: Expr,
}

#[derive(Clone)]
struct Arm {
    mask: Expr,
    inputs: Vec<usize>,
    value: Option<Expr>,
    invalid: bool,
}
#[derive(Clone)]
struct Variant {
    ty: DataType,
    mask: Expr,
    origins: Origins,
    physical: bool,
    nonnull: bool,
    dependencies: Dependencies,
    arms: Vec<Arm>,
}
#[derive(Clone, Copy)]
enum Lazy {
    None,
    Coalesce,
    NullIf,
    And,
    Or,
    Xor,
}
#[derive(Clone)]
enum Kind {
    Base,
    Ast {
        expression: ast::Expr,
        names: Vec<String>,
        lazy: Lazy,
    },
    Append,
    Unified,
}
#[derive(Clone)]
struct Node {
    kind: Kind,
    children: Vec<usize>,
    variants: Vec<Variant>,
    readable: Option<Expr>,
    aggregate_key: Option<String>,
}

struct Compiler<'a> {
    plan: LogicalPlanBuilder,
    scope: &'a mut Bindings,
    session: &'a SessionState,
    context: BindContext<'a>,
    nodes: Vec<Node>,
    stored: bool,
    root: usize,
}

pub(super) fn compile(
    plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    session: &SessionState,
    expression: &ast::Expr,
    context: BindContext<'_>,
) -> Result<(LogicalPlanBuilder, Option<Expr>)> {
    let stored = mixed_operator(expression, scope, false);
    if (!stored && !needs_shared_coalesce(expression))
        || (!context.aggregates && contains_aggregate(expression))
    {
        return Ok((plan, None));
    }
    let key = cache_key(expression, session, plan.schema(), scope, context)?;
    if let Some(cached) = scope.operator_cache.values.get(&key) {
        if cached
            .expression
            .column_refs()
            .iter()
            .all(|column| plan.schema().qualified_field_from_column(column).is_ok())
        {
            return Ok((plan, Some(cached.expression.clone())));
        }
    }
    let mut compiler = Compiler {
        plan,
        scope,
        session,
        context,
        nodes: vec![],
        stored,
        root: usize::MAX,
    };
    let root = compiler.build(expression)?;
    compiler.root = root;
    let variants = compiler.nodes[root].variants.clone();
    let demands = variants
        .iter()
        .map(|variant| variant.mask.clone())
        .collect();
    let values = compiler.emit(root, demands)?;
    let mut ty = DataType::Null;
    for variant in &variants {
        ty = common_value_type(&ty, &variant.ty)?;
    }
    let mut arms = Vec::new();
    let mut dependencies = Dependencies::new();
    for (variant, value) in variants.iter().zip(values) {
        dependencies.extend(variant.dependencies.iter().cloned());
        let value = if value.get_type(compiler.plan.schema())? != ty {
            value.cast_to(&ty, compiler.plan.schema())?
        } else {
            value
        };
        arms.push((Box::new(variant.mask.clone()), Box::new(value)));
    }
    let value = conditional_value(arms, &ty)?;
    let value = if !value.column_refs().is_empty()
        && datafusion::logical_expr::utils::find_aggregate_exprs([&value]).is_empty()
    {
        compiler
            .project(vec![("value", value, dependencies)])?
            .remove(0)
    } else {
        value
    };
    compiler.scope.operator_cache.values.insert(
        key,
        Cached {
            expression: value.clone(),
        },
    );
    Ok((compiler.plan, Some(value)))
}

impl Compiler<'_> {
    fn fresh(&mut self, prefix: &str) -> String {
        loop {
            let name = self.scope.fresh(prefix);
            if !self.scope.contains(&name)
                && !self
                    .context
                    .aliases
                    .is_some_and(|aliases| aliases.contains_key(&name))
            {
                return name;
            }
        }
    }
    fn semantic(&self, expression: &Expr) -> Dependencies {
        let mut result = Dependencies::new();
        for column in expression.column_refs() {
            if let Some(dependencies) = self.scope.operator_dependencies.get(column) {
                result.extend(dependencies.iter().cloned());
                continue;
            }
            let source = column.relation.as_ref().and_then(|relation| {
                self.scope
                    .fragment_cache
                    .iter()
                    .find(|(_, fragments)| {
                        fragments
                            .iter()
                            .any(|fragment| fragment.alias == relation.table())
                    })
                    .map(|(key, _)| key.alias.clone())
            });
            if let Some(alias) = source {
                result.insert(Column::new(
                    Some(TableReference::bare(alias)),
                    crate::graph::ID,
                ));
            } else {
                result.insert(column.clone());
            }
        }
        result
    }
    fn project(&mut self, items: Vec<(&str, Expr, Dependencies)>) -> Result<Vec<Expr>> {
        if items.is_empty() {
            return Ok(Vec::new());
        }
        let mut projected = self
            .plan
            .schema()
            .columns()
            .into_iter()
            .map(Expr::Column)
            .collect::<Vec<_>>();
        let mut columns = Vec::new();
        for (prefix, value, dependencies) in items {
            let name = self.fresh(&format!("op_{prefix}"));
            let column = Column::new_unqualified(&name);
            projected.push(value.alias(name));
            self.scope
                .operator_dependencies
                .insert(column.clone(), dependencies);
            columns.push(Expr::Column(column));
        }
        self.plan = std::mem::replace(&mut self.plan, LogicalPlanBuilder::empty(false))
            .project(projected)?;
        Ok(columns)
    }
    fn finish(
        &mut self,
        kind: Kind,
        children: Vec<usize>,
        variants: Vec<Variant>,
        readable: Option<Expr>,
    ) -> Result<usize> {
        let mut groups: Vec<Variant> = Vec::new();
        for variant in variants {
            if let Some(group) = groups
                .iter_mut()
                .find(|group| group.ty == variant.ty && group.physical == variant.physical)
            {
                group.mask = or(group.mask.clone(), variant.mask);
                group.nonnull &= variant.nonnull;
                group.dependencies.extend(variant.dependencies);
                for (name, frames) in variant.origins {
                    group.origins.entry(name).or_default().extend(frames);
                }
                group.arms.extend(variant.arms);
            } else {
                groups.push(variant);
            }
        }
        let mut definitions = Vec::new();
        let mut indices = Vec::new();
        for (index, group) in groups.iter().enumerate() {
            if !literal_bool(&group.mask, true) {
                indices.push(index);
                definitions.push(("when", group.mask.clone(), self.semantic(&group.mask)));
            }
        }
        let columns = self.project(definitions)?;
        for (index, column) in indices.into_iter().zip(columns) {
            groups[index].mask = column;
        }
        let id = self.nodes.len();
        self.nodes.push(Node {
            kind,
            children,
            variants: groups,
            readable,
            aggregate_key: None,
        });
        Ok(id)
    }
    fn base(&mut self, expression: Expr) -> Result<usize> {
        let variant = Variant {
            ty: expression.get_type(self.plan.schema())?,
            mask: lit(true),
            origins: Origins::new(),
            physical: true,
            nonnull: !expression.nullable(self.plan.schema())?,
            dependencies: self.semantic(&expression),
            arms: vec![Arm {
                mask: lit(true),
                inputs: vec![],
                value: Some(expression),
                invalid: false,
            }],
        };
        self.finish(Kind::Base, vec![], vec![variant], None)
    }
    fn property(&mut self, name: &str, key: &str, original: &ast::Expr) -> Result<usize> {
        if !self.stored {
            return self.base(
                self.context
                    .binder(self.session, self.plan.schema(), self.scope)
                    .bind(original)?,
            );
        }
        let Some(binding) = fragment_binding(self.scope, name) else {
            return self.base(
                self.context
                    .binder(self.session, self.plan.schema(), self.scope)
                    .bind(original)?,
            );
        };
        let (plan, fragments) = attach_fragments(
            std::mem::replace(&mut self.plan, LogicalPlanBuilder::empty(false)),
            self.scope,
            &binding,
        )?;
        self.plan = plan;
        let mut dependencies = Dependencies::new();
        dependencies.insert(Column::new(
            Some(TableReference::bare(binding.alias.clone())),
            crate::graph::ID,
        ));
        let mut readable = binding.ensure_live(lit(true), DataType::Boolean);
        if !self.scope.elements.contains_key(name) {
            if let Some(reference) = self.scope.references.get(name) {
                let scalar = Expr::Column(
                    self.scope
                        .scalars
                        .get(name)
                        .expect("reference column")
                        .clone(),
                );
                readable = and(
                    readable,
                    scalar.clone().is_null().or(reference.guard(scalar)),
                );
            }
        }
        let mut variants = Vec::new();
        for fragment in &fragments {
            let (value, ty, nonnull) = crate::graph::property_field(&fragment.schema, key)
                .map(|field| {
                    (
                        qualified_property(&fragment.schema, Some(&fragment.alias), key),
                        field.data_type().clone(),
                        !field.is_nullable(),
                    )
                })
                .unwrap_or_else(|| (lit(ScalarValue::Null), DataType::Null, false));
            let mask = fragment.column(crate::graph::ID).is_not_null();
            variants.push(Variant {
                ty,
                mask: mask.clone(),
                origins: BTreeMap::from([(
                    binding.alias.clone(),
                    BTreeSet::from([fragment.alias.clone()]),
                )]),
                physical: true,
                nonnull,
                dependencies: dependencies.clone(),
                arms: vec![Arm {
                    mask,
                    inputs: vec![],
                    value: Some(value),
                    invalid: false,
                }],
            });
        }
        let mask = binding.column(crate::graph::ID).is_null();
        variants.push(Variant {
            ty: DataType::Null,
            mask: mask.clone(),
            origins: BTreeMap::from([(binding.alias.clone(), BTreeSet::from(["\0null".into()]))]),
            physical: fragments.is_empty(),
            nonnull: false,
            dependencies,
            arms: vec![Arm {
                mask,
                inputs: vec![],
                value: Some(lit(ScalarValue::Null)),
                invalid: false,
            }],
        });
        self.finish(
            Kind::Base,
            vec![],
            variants,
            (!literal_bool(&readable, true)).then_some(readable),
        )
    }
    fn build(&mut self, expression: &ast::Expr) -> Result<usize> {
        use ast::Expr as E;
        if matches!(expression, E::Function { .. }) {
            let key = cache_key(
                expression,
                self.session,
                self.plan.schema(),
                self.scope,
                self.context,
            )?;
            if let Some(cached) = self.scope.operator_cache.values.get(&key) {
                if cached.expression.column_refs().iter().all(|column| {
                    self.plan
                        .schema()
                        .qualified_field_from_column(column)
                        .is_ok()
                }) {
                    return self.base(cached.expression.clone());
                }
            }
        }
        match expression {
            E::Property { base, key } if matches!(base.as_ref(), E::Identifier(_)) => {
                let E::Identifier(name) = base.as_ref() else {
                    unreachable!()
                };
                self.property(&name.value, &key.value, expression)
            }
            E::Binary { left, op, right } => {
                let a = self.build(left)?;
                let b = self.build(right)?;
                let lazy = match op {
                    ast::BinaryOp::And => Lazy::And,
                    ast::BinaryOp::Or => Lazy::Or,
                    ast::BinaryOp::Xor => Lazy::Xor,
                    _ => Lazy::None,
                };
                self.ast(expression.clone(), vec![a, b], lazy)
            }
            E::Unary { expr, .. }
            | E::IsNull { expr, .. }
            | E::IsUnknown { expr, .. }
            | E::IsTruth { expr, .. }
            | E::Elements { path: expr } => {
                let child = self.build(expr)?;
                self.ast(expression.clone(), vec![child], Lazy::None)
            }
            E::Coalesce(args) if !args.is_empty() => {
                let mut node = self.build(&args[0])?;
                if args.len() == 1 {
                    return self.ast(expression.clone(), vec![node], Lazy::Coalesce);
                }
                for arg in &args[1..] {
                    let right = self.build(arg)?;
                    node = self.ast(
                        E::Coalesce(vec![E::Literal(ast::Literal::Null); 2]),
                        vec![node, right],
                        Lazy::Coalesce,
                    )?;
                }
                Ok(node)
            }
            E::NullIf { left, right } => {
                let a = self.build(left)?;
                let b = self.build(right)?;
                self.ast(expression.clone(), vec![a, b], Lazy::NullIf)
            }
            E::List(args) if !args.is_empty() => {
                let first = self.build(&args[0])?;
                let mut node = self.ast(
                    E::List(vec![E::Literal(ast::Literal::Null)]),
                    vec![first],
                    Lazy::None,
                )?;
                for arg in &args[1..] {
                    let item = self.build(arg)?;
                    node = self.infer(Kind::Append, vec![node, item])?;
                }
                Ok(node)
            }
            E::Function { args, .. }
                if self.context.aggregates
                    && args
                        .first()
                        .is_some_and(|arg| mixed_operator(arg, self.scope, false)) =>
            {
                let key = cache_key(
                    expression,
                    self.session,
                    self.plan.schema(),
                    self.scope,
                    self.context,
                )?;
                let child = self.build(&args[0])?;
                let child = self.unified(child)?;
                let id = self.ast(expression.clone(), vec![child], Lazy::None)?;
                self.nodes[id].aggregate_key = Some(key);
                Ok(id)
            }
            E::NumericFunction { args, .. } => {
                let children = args
                    .iter()
                    .map(|arg| self.build(arg))
                    .collect::<Result<Vec<_>>>()?;
                self.ast(expression.clone(), children, Lazy::None)
            }
            _ => self.base(
                self.context
                    .binder(self.session, self.plan.schema(), self.scope)
                    .bind(expression)?,
            ),
        }
    }
    /// Aggregate arguments span rows, so collapse their stored variants before
    /// binding the aggregate rather than aggregating each fragment separately.
    fn unified(&mut self, child: usize) -> Result<usize> {
        let mut ty = DataType::Null;
        let mut dependencies = Dependencies::new();
        for variant in &self.nodes[child].variants {
            ty = common_value_type(&ty, &variant.ty)?;
            dependencies.extend(variant.dependencies.iter().cloned());
        }
        self.finish(
            Kind::Unified,
            vec![child],
            vec![Variant {
                ty,
                mask: lit(true),
                origins: Origins::new(),
                physical: true,
                nonnull: false,
                dependencies,
                arms: vec![],
            }],
            None,
        )
    }
    fn ast(
        &mut self,
        mut expression: ast::Expr,
        children: Vec<usize>,
        lazy: Lazy,
    ) -> Result<usize> {
        use ast::Expr as E;
        let names = (0..children.len())
            .map(|_| self.fresh("operand"))
            .collect::<Vec<_>>();
        let identifier = |index: usize| E::Identifier(ast::Identifier::new(&names[index]));
        match &mut expression {
            E::Binary { left, right, .. } | E::NullIf { left, right } => {
                **left = identifier(0);
                **right = identifier(1);
            }
            E::Unary { expr, .. }
            | E::IsNull { expr, .. }
            | E::IsUnknown { expr, .. }
            | E::IsTruth { expr, .. }
            | E::Elements { path: expr } => **expr = identifier(0),
            E::Function { args, .. } => args[0] = identifier(0),
            E::Coalesce(args) | E::List(args) | E::NumericFunction { args, .. } => {
                *args = (0..names.len()).map(identifier).collect()
            }
            _ => unreachable!("stored operator node"),
        }
        self.infer(
            Kind::Ast {
                expression,
                names,
                lazy,
            },
            children,
        )
    }
    fn bind(&self, kind: &Kind, args: Vec<Expr>) -> Result<Expr> {
        match kind {
            Kind::Ast {
                expression, names, ..
            } => {
                let mut aliases = self.context.aliases.cloned().unwrap_or_default();
                for (name, value) in names.iter().zip(args) {
                    aliases.insert(name.clone(), value);
                }
                self.context
                    .binder(self.session, self.plan.schema(), self.scope)
                    .aliases(&aliases)
                    .bind(expression)
            }
            Kind::Append => {
                let DataType::List(field) = args[0].get_type(self.plan.schema())? else {
                    return Err(Error::InvalidQuery(
                        "incompatible stored list elements".into(),
                    ));
                };
                let element =
                    common_value_type(field.data_type(), &args[1].get_type(self.plan.schema())?)?;
                let list = args[0].clone().cast_to(
                    &DataType::new_list(element.clone(), true),
                    self.plan.schema(),
                )?;
                let item = args[1].clone().cast_to(&element, self.plan.schema())?;
                Ok(datafusion::functions_nested::expr_fn::array_append(
                    list, item,
                ))
            }
            Kind::Base | Kind::Unified => unreachable!(),
        }
    }
    fn infer(&mut self, kind: Kind, children: Vec<usize>) -> Result<usize> {
        let operands = children
            .iter()
            .map(|id| self.nodes[*id].variants.clone())
            .collect::<Vec<_>>();
        let mut combinations = vec![vec![]];
        for variants in &operands {
            combinations = combinations
                .into_iter()
                .flat_map(|prefix| {
                    (0..variants.len()).map(move |index| {
                        let mut indices = prefix.clone();
                        indices.push(index);
                        indices
                    })
                })
                .collect();
        }
        let mut variants = Vec::new();
        let mut physical = false;
        let mut supported = false;
        let mut error = None;
        for indices in combinations {
            let mut origins = Origins::new();
            let mut mask = lit(true);
            let mut dependencies = Dependencies::new();
            let mut actual = true;
            let mut possible = true;
            for (operand, index) in operands.iter().zip(&indices) {
                let variant = &operand[*index];
                if let Some(merged) = intersect(&origins, &variant.origins) {
                    origins = merged;
                } else {
                    possible = false;
                    break;
                }
                mask = and(mask, variant.mask.clone());
                actual &= variant.physical;
                dependencies.extend(variant.dependencies.iter().cloned());
            }
            if !possible {
                continue;
            }
            physical |= actual;
            let dummy = operands
                .iter()
                .zip(&indices)
                .map(|(operand, index)| Ok(lit(ScalarValue::try_from(&operand[*index].ty)?)))
                .collect::<Result<Vec<_>>>()?;
            let (ty, invalid) = match self
                .bind(&kind, dummy)
                .and_then(|value| value.get_type(self.plan.schema()).map_err(Error::from))
            {
                Ok(ty) => {
                    supported |= actual;
                    (ty, false)
                }
                Err(err @ (Error::InvalidQuery(_) | Error::UnsupportedFeature(_))) => {
                    if error.is_none() {
                        error = Some(err);
                    }
                    (DataType::Null, true)
                }
                Err(err) => return Err(err),
            };
            let nonnull = if invalid {
                false
            } else {
                node_nonnull(&kind, &operands, &indices)
            };
            let value = invalid.then(|| lit(ScalarValue::Null));
            variants.push(Variant {
                ty,
                mask: mask.clone(),
                origins,
                physical: actual,
                nonnull,
                dependencies,
                arms: vec![Arm {
                    mask,
                    inputs: indices,
                    value,
                    invalid,
                }],
            });
        }
        if physical && !supported {
            return Err(error.unwrap_or_else(|| {
                Error::InvalidQuery("operator does not match a stored type".into())
            }));
        }
        self.finish(kind, children, variants, None)
    }
    fn emit(&mut self, id: usize, demands: Vec<Expr>) -> Result<Vec<Expr>> {
        let node = self.nodes[id].clone();
        let cacheable = demands.len() == 1 && literal_bool(&demands[0], true);
        if matches!(node.kind, Kind::Unified) {
            let child = node.children[0];
            let variants = self.nodes[child].variants.clone();
            let needed = variants
                .iter()
                .map(|variant| and(demands[0].clone(), variant.mask.clone()))
                .collect();
            let values = self.emit(child, needed)?;
            let ty = &node.variants[0].ty;
            let arms = variants
                .iter()
                .zip(values)
                .map(|(variant, value)| {
                    Ok((
                        Box::new(variant.mask.clone()),
                        Box::new(value.cast_to(ty, self.plan.schema())?),
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            let value = Expr::Case(datafusion::logical_expr::expr::Case::new(
                None,
                arms,
                Some(Box::new(lit(ScalarValue::try_from(ty)?))),
            ));
            let value =
                if datafusion::logical_expr::utils::find_aggregate_exprs([&value]).is_empty() {
                    let mut dependencies = node.variants[0].dependencies.clone();
                    dependencies.extend(self.semantic(&demands[0]));
                    self.project(vec![("argument", value, dependencies)])?
                        .remove(0)
                } else {
                    value
                };
            return Ok(vec![value]);
        }
        let mut values: Vec<Vec<Expr>> = Vec::new();
        let lazy = match node.kind {
            Kind::Ast { lazy, .. } => lazy,
            _ => Lazy::None,
        };
        for (position, child) in node.children.iter().enumerate() {
            let mut needed = vec![lit(false); self.nodes[*child].variants.len()];
            for (variant, demand) in node.variants.iter().zip(&demands) {
                for arm in &variant.arms {
                    if arm.invalid {
                        continue;
                    }
                    let mut active = and(demand.clone(), arm.mask.clone());
                    if position == 1 && !matches!(lazy, Lazy::None) {
                        let first = &values[0][arm.inputs[0]];
                        let info = &self.nodes[node.children[0]].variants[arm.inputs[0]];
                        let aggregate =
                            !datafusion::logical_expr::utils::find_aggregate_exprs([first])
                                .is_empty();
                        let condition = if aggregate && !matches!(lazy, Lazy::Coalesce) {
                            lit(true)
                        } else {
                            match lazy {
                                Lazy::Coalesce if info.nonnull => lit(false),
                                Lazy::Coalesce
                                    if !datafusion::logical_expr::utils::find_aggregate_exprs(
                                        [first],
                                    )
                                    .is_empty() =>
                                {
                                    lit(true)
                                }
                                Lazy::Coalesce => first.clone().is_null(),
                                Lazy::NullIf | Lazy::Xor => first.clone().is_not_null(),
                                Lazy::And => first.clone().is_not_false(),
                                Lazy::Or => first.clone().is_not_true(),
                                Lazy::None => lit(true),
                            }
                        };
                        active = and(active, condition);
                    }
                    let index = arm.inputs[position];
                    needed[index] = or(needed[index].clone(), active);
                }
            }
            let mut indices = Vec::new();
            let mut projected = Vec::new();
            for (index, demand) in needed.iter().enumerate() {
                if !literal_bool(demand, true) && !literal_bool(demand, false) {
                    indices.push(index);
                    projected.push(("demand", demand.clone(), self.semantic(demand)));
                }
            }
            for (index, column) in indices.into_iter().zip(self.project(projected)?) {
                needed[index] = column;
            }
            values.push(self.emit(*child, needed)?);
        }
        if let Some(readable) = node.readable {
            let active = demands.iter().cloned().fold(lit(false), or);
            if !literal_bool(&active, false) {
                let check = Expr::Case(datafusion::logical_expr::expr::Case::new(
                    None,
                    vec![(Box::new(active), Box::new(readable))],
                    Some(Box::new(lit(true))),
                ));
                self.plan = std::mem::replace(&mut self.plan, LogicalPlanBuilder::empty(false))
                    .filter(check)?;
            }
        }
        let mut result = Vec::new();
        let mut projected = Vec::new();
        let mut indices = Vec::new();
        for (index, (variant, demand)) in node.variants.iter().zip(demands).enumerate() {
            let mut arms = Vec::new();
            for arm in &variant.arms {
                let active = and(demand.clone(), arm.mask.clone());
                if literal_bool(&active, false) {
                    continue;
                }
                let value = if let Some(value) = &arm.value {
                    value.clone()
                } else {
                    let args = arm
                        .inputs
                        .iter()
                        .enumerate()
                        .map(|(child, index)| values[child][*index].clone())
                        .collect();
                    self.bind(&node.kind, args)?
                };
                arms.push((Box::new(active), Box::new(value)));
            }
            let mut value = conditional_value(arms, &variant.ty)?;
            let aggregate =
                !datafusion::logical_expr::utils::find_aggregate_exprs([&value]).is_empty();
            if value.column_refs().is_empty() && !aggregate {
                use datafusion::optimizer::simplify_expressions::{
                    ExprSimplifier, SimplifyContext,
                };
                value = ExprSimplifier::new(SimplifyContext::default()).simplify(value)?;
            }
            let computed = !matches!(node.kind, Kind::Base)
                && id != self.root
                && !matches!(value, Expr::Literal(..));
            if (computed || !value.column_refs().is_empty()) && !aggregate {
                indices.push(index);
                let mut dependencies = variant.dependencies.clone();
                dependencies.extend(self.semantic(&demand));
                projected.push(("typed", value, dependencies));
                result.push(lit(ScalarValue::Null));
            } else {
                result.push(value);
            }
        }
        for (index, column) in indices.into_iter().zip(self.project(projected)?) {
            result[index] = column;
        }
        if cacheable && result.len() == 1 {
            if let Some(key) = node.aggregate_key {
                if !datafusion::logical_expr::utils::find_aggregate_exprs([&result[0]]).is_empty() {
                    self.scope.operator_cache.values.insert(
                        key,
                        Cached {
                            expression: result[0].clone(),
                        },
                    );
                }
            }
        }
        Ok(result)
    }
}

fn conditional_value(mut arms: Vec<(Box<Expr>, Box<Expr>)>, ty: &DataType) -> Result<Expr> {
    if arms.len() == 1 && literal_bool(&arms[0].0, true) {
        return Ok(*arms.pop().expect("single arm").1);
    }
    let null = lit(ScalarValue::try_from(ty)?);
    if arms.is_empty() {
        return Ok(null);
    }
    Ok(Expr::Case(datafusion::logical_expr::expr::Case::new(
        None,
        arms,
        Some(Box::new(null)),
    )))
}

fn node_nonnull(kind: &Kind, operands: &[Vec<Variant>], indices: &[usize]) -> bool {
    use ast::Expr as E;
    let all = operands
        .iter()
        .zip(indices)
        .all(|(variants, index)| variants[*index].nonnull);
    match kind {
        Kind::Append => true,
        Kind::Ast {
            expression: E::IsNull { .. } | E::IsUnknown { .. } | E::IsTruth { .. } | E::List(_),
            ..
        } => true,
        Kind::Ast {
            expression: E::Coalesce(_),
            ..
        } => operands
            .iter()
            .zip(indices)
            .any(|(variants, index)| variants[*index].nonnull),
        Kind::Ast {
            expression: E::NullIf { .. },
            ..
        } => false,
        Kind::Ast {
            expression:
                E::Binary {
                    op:
                        ast::BinaryOp::Eq
                        | ast::BinaryOp::Neq
                        | ast::BinaryOp::Lt
                        | ast::BinaryOp::Le
                        | ast::BinaryOp::Gt
                        | ast::BinaryOp::Ge,
                    ..
                },
            ..
        } if operands.iter().zip(indices).any(|(variants, index)| {
            matches!(
                variants[*index].ty,
                DataType::List(_) | DataType::LargeList(_)
            )
        }) =>
        {
            false
        }
        _ => all,
    }
}
fn intersect(left: &Origins, right: &Origins) -> Option<Origins> {
    let mut result = left.clone();
    for (name, frames) in right {
        if let Some(current) = result.get_mut(name) {
            *current = current.intersection(frames).cloned().collect();
            if current.is_empty() {
                return None;
            }
        } else {
            result.insert(name.clone(), frames.clone());
        }
    }
    Some(result)
}
fn literal_bool(expr: &Expr, expected: bool) -> bool {
    matches!(expr,Expr::Literal(ScalarValue::Boolean(Some(value)),_) if *value==expected)
}
fn and(left: Expr, right: Expr) -> Expr {
    if literal_bool(&left, false) || literal_bool(&right, false) {
        lit(false)
    } else if literal_bool(&left, true) {
        right
    } else if literal_bool(&right, true) || left == right {
        left
    } else {
        left.and(right)
    }
}
fn or(left: Expr, right: Expr) -> Expr {
    if literal_bool(&left, true) || literal_bool(&right, true) {
        lit(true)
    } else if literal_bool(&left, false) {
        right
    } else if literal_bool(&right, false) || left == right {
        left
    } else {
        left.or(right)
    }
}
pub(super) fn contains_aggregate(expr: &ast::Expr) -> bool {
    matches!(expr, ast::Expr::Function { .. }) || children(expr).into_iter().any(contains_aggregate)
}
fn mixed_operator(expr: &ast::Expr, scope: &Bindings, inside: bool) -> bool {
    let inside = inside
        || matches!(
            expr,
            ast::Expr::Binary {
                op: ast::BinaryOp::Add
                    | ast::BinaryOp::Sub
                    | ast::BinaryOp::Mul
                    | ast::BinaryOp::Div
                    | ast::BinaryOp::Concat,
                ..
            } | ast::Expr::Unary {
                op: ast::UnaryOp::Pos | ast::UnaryOp::Neg,
                ..
            }
        );
    if let ast::Expr::Property { base, key } = expr {
        if let ast::Expr::Identifier(name) = base.as_ref() {
            if inside {
                if let Some(binding) = fragment_binding(scope, &name.value) {
                    if let Some(data) = scope.sources.get(&binding.graph) {
                        let types = element_tables(data, &binding)
                            .into_iter()
                            .filter_map(|table| {
                                crate::graph::property_field(&table.schema, &key.value)
                                    .map(|field| field.data_type().clone())
                            })
                            .collect::<Vec<_>>();
                        return types.iter().any(|ty| types.first() != Some(ty));
                    }
                }
            }
        }
    }
    children(expr)
        .into_iter()
        .any(|child| mixed_operator(child, scope, inside))
}
fn cache_key(
    expr: &ast::Expr,
    session: &SessionState,
    schema: &DFSchema,
    scope: &Bindings,
    context: BindContext<'_>,
) -> Result<String> {
    use ast::Expr as E;
    let child = |expr: &ast::Expr| cache_key(expr, session, schema, scope, context);
    Ok(match expr {
        E::Property { base, key } => {
            if let E::Identifier(name) = base.as_ref() {
                if let Some(binding) = fragment_binding(scope, &name.value) {
                    let snapshot = scope
                        .sources
                        .get(&binding.graph)
                        .map(|data| std::sync::Arc::as_ptr(data) as usize);
                    return Ok(format!(
                        "property:{}:{binding:?}:{snapshot:?}",
                        key.value.to_ascii_lowercase()
                    ));
                }
            }
            format!("{:?}", context.binder(session, schema, scope).bind(expr)?)
        }
        E::Binary { left, op, right } => {
            format!("binary:{op:?}({},{})", child(left)?, child(right)?)
        }
        E::Unary { op, expr } => format!("unary:{op:?}({})", child(expr)?),
        E::IsNull { expr, negated } => format!("null:{negated}({})", child(expr)?),
        E::IsUnknown { expr, negated } => format!("unknown:{negated}({})", child(expr)?),
        E::IsTruth {
            expr,
            negated,
            value,
        } => format!("truth:{negated}:{value}({})", child(expr)?),
        E::NullIf { left, right } => format!("nullif({},{})", child(left)?, child(right)?),
        E::Coalesce(args) | E::List(args) | E::NumericFunction { args, .. } => {
            let values = args.iter().map(child).collect::<Result<Vec<_>>>()?;
            let tag = match expr {
                E::Coalesce(_) => "coalesce".into(),
                E::List(_) => "list".into(),
                E::NumericFunction { function, .. } => format!("numeric:{function:?}"),
                _ => unreachable!(),
            };
            format!("{tag}:{}", key_arguments(values))
        }
        E::Wildcard => "wildcard".into(),
        E::Function {
            name,
            quantifier,
            args,
        } => format!(
            "aggregate:{}:{quantifier:?}:{:?}",
            name.value.to_ascii_lowercase(),
            key_arguments(args.iter().map(child).collect::<Result<Vec<_>>>()?)
        ),
        E::Elements { path } => format!("elements({})", child(path)?),
        _ => format!(
            "bound:{:?}",
            context.binder(session, schema, scope).bind(expr)?
        ),
    })
}

// Length framing keeps recursive keys unambiguous without repeatedly escaping
// child strings, which itself doubles their size at each nesting level.
fn key_arguments(values: Vec<String>) -> String {
    values
        .into_iter()
        .map(|value| format!("{}:{value}", value.len()))
        .collect::<Vec<_>>()
        .join("")
}
