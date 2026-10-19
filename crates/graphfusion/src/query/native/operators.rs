//! Evaluate mixed-property operators without replacing element metadata bindings.
use super::super::expressions::PropertyValues;
use super::*;

struct Source {
    name: String,
    binding: ElementBinding,
    fragments: Vec<Fragment>,
}

#[derive(Default)]
struct Viability {
    supported: bool,
    error: Option<String>,
}

pub(super) fn compile(
    mut plan: LogicalPlanBuilder,
    scope: &mut Bindings,
    session: &SessionState,
    expression: &ast::Expr,
    context: BindContext<'_>,
) -> Result<(LogicalPlanBuilder, Option<Expr>)> {
    if !mixed_operator(expression, scope, false) || contains_aggregate(expression) {
        return Ok((plan, None));
    }
    // Boolean branches have independent row-dependent operands. Keep them in
    // the shared composition path rather than expanding a Cartesian CASE tree.
    if matches!(
        expression,
        ast::Expr::Binary {
            op: ast::BinaryOp::And | ast::BinaryOp::Or | ast::BinaryOp::Xor,
            ..
        }
    ) {
        return Ok((plan, None));
    }
    let mut names = BTreeSet::new();
    property_bases(expression, &mut names);
    let mut sources = Vec::new();
    for name in names {
        let Some(binding) = fragment_binding(scope, &name) else {
            continue;
        };
        let (next, fragments) = attach_fragments(plan, scope, &binding)?;
        plan = next;
        sources.push(Source {
            name,
            binding,
            fragments,
        });
    }
    let keys = property_keys(expression);
    let mut contexts: Vec<Vec<Option<Fragment>>> = vec![vec![]];
    for source in &sources {
        let mut next = Vec::new();
        for prefix in contexts {
            for fragment in source.fragments.iter().cloned().map(Some).chain([None]) {
                let mut item = prefix.clone();
                item.push(fragment);
                next.push(item);
            }
        }
        contexts = next;
    }
    let mut viability = BTreeMap::new();
    let mut arms = Vec::new();
    for fragments in contexts {
        let mut properties = PropertyValues::new();
        let mut condition = lit(true);
        for (source, fragment) in sources.iter().zip(&fragments) {
            let selected = fragment.as_ref().map_or_else(
                || source.binding.column(crate::graph::ID).is_null(),
                |fragment| fragment.column(crate::graph::ID).is_not_null(),
            );
            condition = condition.and(selected);
            let mut values = CaseFoldMap::new();
            for (name, key) in &keys {
                if !source.name.eq_ignore_ascii_case(name) {
                    continue;
                }
                let (value, data_type) = fragment
                    .as_ref()
                    .and_then(|fragment| {
                        crate::graph::property_field(&fragment.schema, key).map(|field| {
                            (
                                qualified_property(&fragment.schema, Some(&fragment.alias), key),
                                field.data_type().clone(),
                            )
                        })
                    })
                    .unwrap_or_else(|| (lit(ScalarValue::Null), DataType::Null));
                let value = source.binding.ensure_live(value, data_type);
                values.insert(key.clone(), value);
            }
            properties.insert(source.name.clone(), values);
        }
        let binder = context
            .binder(session, plan.schema(), scope)
            .property_values(&properties);
        let rewritten = rewrite(
            expression,
            &binder,
            plan.schema(),
            &sources,
            &fragments,
            &mut viability,
        )?;
        arms.push((condition, binder.bind(&rewritten)?));
    }
    if let Some(failed) = viability.values().find(|item| !item.supported) {
        return Err(Error::InvalidQuery(
            failed
                .error
                .clone()
                .unwrap_or_else(|| "operator does not match a stored type".into()),
        ));
    }
    let mut data_type = DataType::Null;
    for (_, value) in &arms {
        let incoming = value.get_type(plan.schema())?;
        if data_type == DataType::Null {
            data_type = incoming;
        } else if incoming != DataType::Null && incoming != data_type {
            if incoming.is_numeric() && data_type.is_numeric() {
                data_type = DataType::Float64;
            } else {
                return Err(Error::InvalidQuery(format!(
                    "incompatible stored operator results {data_type} and {incoming}"
                )));
            }
        }
    }
    let arms = arms
        .into_iter()
        .map(|(condition, value)| {
            let value = if value.get_type(plan.schema())? != data_type {
                value.cast_to(&data_type, plan.schema())?
            } else {
                value
            };
            Ok((Box::new(condition), Box::new(value)))
        })
        .collect::<Result<Vec<_>>>()?;
    let otherwise = lit(ScalarValue::try_from(&data_type)?);
    let mut value = Expr::Case(datafusion::logical_expr::expr::Case::new(
        None,
        arms,
        Some(Box::new(otherwise)),
    ));
    for source in &sources {
        value = source.binding.ensure_live(value, data_type.clone());
        if !scope.elements.contains_key(&source.name) {
            if let Some(reference) = scope.references.get(&source.name) {
                let scalar = Expr::Column(
                    scope
                        .scalars
                        .get(&source.name)
                        .expect("reference column")
                        .clone(),
                );
                let live = scalar.clone().is_null().or(reference.guard(scalar));
                value = Expr::Case(datafusion::logical_expr::expr::Case::new(
                    None,
                    vec![(Box::new(live), Box::new(value))],
                    Some(Box::new(lit(ScalarValue::try_from(&data_type)?))),
                ));
            }
        }
    }
    Ok((plan, Some(value)))
}

fn contains_aggregate(expr: &ast::Expr) -> bool {
    matches!(expr, ast::Expr::Function { .. }) || children(expr).into_iter().any(contains_aggregate)
}

fn mixed_operator(expr: &ast::Expr, scope: &Bindings, in_operator: bool) -> bool {
    let in_operator = in_operator
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
            if in_operator {
                if let Some(binding) = fragment_binding(scope, &name.value) {
                    if let Some(data) = scope.sources.get(&binding.graph) {
                        let types: Vec<_> = element_tables(data, &binding)
                            .into_iter()
                            .filter_map(|table| {
                                crate::graph::property_field(&table.schema, &key.value)
                                    .map(|field| field.data_type().clone())
                            })
                            .collect();
                        return types.iter().any(|ty| types.first() != Some(ty));
                    }
                }
            }
        }
    }
    children(expr)
        .into_iter()
        .any(|child| mixed_operator(child, scope, in_operator))
}

fn property_keys(expr: &ast::Expr) -> Vec<(String, String)> {
    let mut keys = Vec::new();
    fn visit(expr: &ast::Expr, keys: &mut Vec<(String, String)>) {
        if let ast::Expr::Property { base, key } = expr {
            if let ast::Expr::Identifier(name) = base.as_ref() {
                if !keys.iter().any(|(n, k)| {
                    n.eq_ignore_ascii_case(&name.value) && k.eq_ignore_ascii_case(&key.value)
                }) {
                    keys.push((name.value.clone(), key.value.clone()));
                }
            }
        }
        for child in children(expr) {
            visit(child, keys);
        }
    }
    visit(expr, &mut keys);
    keys
}

fn record(
    expr: &ast::Expr,
    supported: bool,
    error: String,
    sources: &[Source],
    fragments: &[Option<Fragment>],
    viability: &mut BTreeMap<usize, Viability>,
) {
    let mut dependencies = BTreeSet::new();
    property_bases(expr, &mut dependencies);
    if sources.iter().zip(fragments).any(|(source, fragment)| {
        fragment.is_none()
            && !source.fragments.is_empty()
            && dependencies
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&source.name))
    }) {
        return;
    }
    let entry = viability
        .entry(expr as *const ast::Expr as usize)
        .or_default();
    entry.supported |= supported;
    if !supported && entry.error.is_none() {
        entry.error = Some(error);
    }
}

fn rewrite(
    expr: &ast::Expr,
    binder: &Binder<'_>,
    schema: &DFSchema,
    sources: &[Source],
    fragments: &[Option<Fragment>],
    viability: &mut BTreeMap<usize, Viability>,
) -> Result<ast::Expr> {
    use ast::Expr as E;
    let mut next = expr.clone();
    match (&mut next, expr) {
        (
            E::Binary { left, op, right },
            E::Binary {
                left: original_left,
                right: original_right,
                ..
            },
        ) => {
            **left = rewrite(original_left, binder, schema, sources, fragments, viability)?;
            **right = rewrite(
                original_right,
                binder,
                schema,
                sources,
                fragments,
                viability,
            )?;
            let a = binder.bind(left)?.get_type(schema)?;
            let b = binder.bind(right)?.get_type(schema)?;
            let numeric = |ty: &DataType| ty.is_numeric() || *ty == DataType::Null;
            let string = |ty: &DataType| {
                matches!(
                    ty,
                    DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View | DataType::Null
                )
            };
            use ast::BinaryOp as B;
            let supported = match op {
                B::Add | B::Sub | B::Mul | B::Div => numeric(&a) && numeric(&b),
                B::Concat => string(&a) && string(&b),
                B::Eq | B::Neq | B::Lt | B::Le | B::Gt | B::Ge => comparable(&a, &b),
                B::And | B::Or | B::Xor => {
                    matches!(a, DataType::Boolean | DataType::Null)
                        && matches!(b, DataType::Boolean | DataType::Null)
                }
            };
            record(
                expr,
                supported,
                format!("operator {op:?} cannot apply to stored types {a} and {b}"),
                sources,
                fragments,
                viability,
            );
            if !supported {
                return Ok(E::Literal(ast::Literal::Null));
            }
        }
        (E::Unary { op, expr: child }, E::Unary { expr: original, .. }) => {
            **child = rewrite(original, binder, schema, sources, fragments, viability)?;
            let ty = binder.bind(child)?.get_type(schema)?;
            let supported = match op {
                ast::UnaryOp::Not => matches!(ty, DataType::Boolean | DataType::Null),
                _ => ty.is_numeric() || ty == DataType::Null,
            };
            record(
                expr,
                supported,
                format!("operator {op:?} cannot apply to stored type {ty}"),
                sources,
                fragments,
                viability,
            );
            if !supported {
                return Ok(E::Literal(ast::Literal::Null));
            }
        }
        (
            E::IsNull { expr: child, .. }
            | E::IsUnknown { expr: child, .. }
            | E::IsTruth { expr: child, .. }
            | E::Elements { path: child },
            E::IsNull { expr: original, .. }
            | E::IsUnknown { expr: original, .. }
            | E::IsTruth { expr: original, .. }
            | E::Elements { path: original },
        ) => {
            **child = rewrite(original, binder, schema, sources, fragments, viability)?;
        }
        (
            E::Coalesce(args)
            | E::List(args)
            | E::Function { args, .. }
            | E::NumericFunction { args, .. },
            E::Coalesce(original)
            | E::List(original)
            | E::Function { args: original, .. }
            | E::NumericFunction { args: original, .. },
        ) => {
            *args = original
                .iter()
                .map(|child| rewrite(child, binder, schema, sources, fragments, viability))
                .collect::<Result<Vec<_>>>()?;
        }
        (
            E::NullIf { left, right },
            E::NullIf {
                left: original_left,
                right: original_right,
            },
        ) => {
            **left = rewrite(original_left, binder, schema, sources, fragments, viability)?;
            **right = rewrite(
                original_right,
                binder,
                schema,
                sources,
                fragments,
                viability,
            )?;
        }
        _ => {}
    }
    Ok(next)
}
