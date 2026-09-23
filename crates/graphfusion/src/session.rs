use crate::{
    catalog::*,
    gql as ast,
    transaction::StatementTxn,
    types::{GraphDefinition, ValueType},
    Database, Error, Result,
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Boolean(bool),
    Integer(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    List(Vec<Value>),
    Record(BTreeMap<String, Value>),
    Graph(ObjectId),
    GraphType(ObjectId),
    Schema(ObjectId),
    BindingTable(Vec<BTreeMap<String, Value>>),
}
#[derive(Clone, Debug, PartialEq)]
pub struct Parameter {
    pub value: Value,
    pub declared_type: Option<ValueType>,
}
#[derive(Clone, Debug, PartialEq)]
pub struct SessionState {
    pub current_schema: ObjectId,
    pub home_schema: ObjectId,
    pub current_graph: Option<ObjectId>,
    pub home_graph: Option<ObjectId>,
    pub time_zone: String,
    pub parameters: BTreeMap<String, Parameter>,
    pub closed: bool,
}
impl Default for SessionState {
    fn default() -> Self {
        Self {
            current_schema: MAIN_SCHEMA,
            home_schema: MAIN_SCHEMA,
            current_graph: None,
            home_graph: None,
            time_zone: "UTC".into(),
            parameters: BTreeMap::new(),
            closed: false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatementResult {
    pub commit_seq: CommitSeq,
    pub affected_objects: usize,
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExecutionResult {
    pub statements: Vec<StatementResult>,
}
#[derive(Debug)]
pub struct Session {
    db: Database,
    state: SessionState,
    initial: SessionState,
}

impl Session {
    pub(crate) fn new(db: Database) -> Self {
        let state = SessionState::default();
        Self {
            db,
            initial: state.clone(),
            state,
        }
    }
    pub fn state(&self) -> &SessionState {
        &self.state
    }
    /// Executes one read-only GQL query through DataFusion and materializes Arrow results.
    /// Catalog/session commands continue to use `execute`.
    pub async fn query(&mut self, input: &str) -> Result<crate::QueryResult> {
        crate::query::execute(&self.db, &self.state, input).await
    }
    /// Atomically replaces the current open graph with validated Arrow tables.
    /// Currently available for in-memory databases; durable imports require Parquet.
    pub fn replace_graph_data(&mut self, data: crate::graph::GraphData) -> Result<CommitSeq> {
        if self.state.closed {
            return Err(Error::SessionClosed);
        }
        let graph = self
            .state
            .current_graph
            .ok_or_else(|| Error::InvalidReference("current graph is unset".into()))?;
        let mut tx = StatementTxn::begin(&self.db)?;
        tx.replace_graph_data(graph, data)?;
        tx.commit()
    }
    /// Sets a driver-provided value. References are checked in the snapshot of each use.
    pub fn set_parameter(&mut self, name: impl Into<String>, value: Value) -> Result<()> {
        if self.state.closed {
            return Err(Error::SessionClosed);
        }
        self.state.parameters.insert(
            name.into(),
            Parameter {
                value,
                declared_type: None,
            },
        );
        Ok(())
    }
    /// Top-level statements commit separately. On an error, earlier statements remain committed.
    /// NEXT continuations and linear catalog statements share one atomic internal transaction.
    pub fn execute(&mut self, input: &str) -> Result<ExecutionResult> {
        if self.state.closed {
            return Err(Error::SessionClosed);
        }
        let program = ast::parse(input)?;
        if program.statements.iter().any(contains_transaction) {
            return Err(Error::UnsupportedFeature(
                "explicit transactions; statements auto-commit".into(),
            ));
        }
        if !program.definitions.is_empty() || program.at_schema.is_some() {
            return Err(Error::UnsupportedFeature(
                "procedure binding definitions".into(),
            ));
        }
        let mut results = ExecutionResult::default();
        let mut index = 0;
        while index < program.statements.len() {
            if self.state.closed {
                return Err(Error::SessionClosed);
            }
            let mut tx = StatementTxn::begin(&self.db)?;
            let mut next = self.state.clone();
            let mut affected = execute_statement(
                &mut tx,
                &mut next,
                &self.initial,
                &program.statements[index],
            )?;
            index += 1;
            while index < program.statements.len()
                && matches!(program.statements[index], ast::Statement::Next(_))
            {
                affected += execute_statement(
                    &mut tx,
                    &mut next,
                    &self.initial,
                    &program.statements[index],
                )?;
                index += 1;
            }
            let commit_seq = tx.commit()?;
            self.state = next;
            results.statements.push(StatementResult {
                commit_seq,
                affected_objects: affected,
            });
        }
        Ok(results)
    }
}

fn contains_transaction(statement: &ast::Statement) -> bool {
    match statement {
        ast::Statement::StartTransaction(_)
        | ast::Statement::Commit(_)
        | ast::Statement::Rollback(_) => true,
        ast::Statement::LinearCatalog(items) => items.iter().any(contains_transaction),
        ast::Statement::Next(next) => contains_transaction(&next.statement),
        // Query and procedure bodies are rejected without effects by this catalog-only executor.
        _ => false,
    }
}

fn execute_statement(
    tx: &mut StatementTxn,
    session: &mut SessionState,
    initial: &SessionState,
    statement: &ast::Statement,
) -> Result<usize> {
    use ast::Statement as S;
    if session.closed {
        return Err(Error::SessionClosed);
    }
    match statement {
        S::LinearCatalog(items) => {
            let mut count = 0;
            for item in items {
                count += execute_statement(tx, session, initial, item)?;
            }
            Ok(count)
        }
        S::Next(next) => {
            if next.yield_clause.is_some() {
                return Err(Error::UnsupportedFeature("NEXT YIELD bindings".into()));
            }
            execute_statement(tx, session, initial, &next.statement)
        }
        S::CreateSchema(s) => {
            let (parent, name) = target(tx, session, Name::from(&s.name), ObjectKind::Schema)?;
            if existing(
                tx,
                parent,
                ObjectKind::Schema,
                &name,
                s.if_not_exists,
                false,
            )? {
                return Ok(0);
            }
            tx.create(parent, &name, ObjectDefinition::Schema)?;
            Ok(1)
        }
        S::DropSchema(s) => drop_named(
            tx,
            session,
            Name::from(&s.name),
            ObjectKind::Schema,
            s.if_exists,
        ),
        S::CreateGraph(s) => {
            let (parent, name) = target(tx, session, Name::from(&s.name), ObjectKind::Graph)?;
            if s.or_replace && s.if_not_exists {
                return Err(Error::InvalidDefinition(
                    "OR REPLACE and IF NOT EXISTS are mutually exclusive".into(),
                ));
            }
            if existing(
                tx,
                parent,
                ObjectKind::Graph,
                &name,
                s.if_not_exists,
                s.or_replace,
            )? {
                return Ok(0);
            }
            if s.source.is_some() {
                return Err(Error::UnsupportedFeature("AS COPY OF graph data".into()));
            }
            let shape = match &s.graph_type {
                None | Some(ast::CreateGraphType::Any { .. }) => GraphShape::Open,
                Some(ast::CreateGraphType::Nested { body, .. }) => {
                    GraphShape::Inline(GraphDefinition::bind(body)?)
                }
                Some(ast::CreateGraphType::Named { name, .. }) => {
                    GraphShape::Named(resolve_type(tx, session, name)?)
                }
                Some(ast::CreateGraphType::Like(graph)) => graph_shape_copy(tx, session, graph)?,
            };
            if s.or_replace {
                if let Some(old) = tx.lookup(parent, ObjectKind::Graph, &name) {
                    tx.drop_object(old.id)?;
                }
            }
            tx.create_graph(parent, &name, shape)?;
            Ok(1)
        }
        S::DropGraph(s) => drop_named(
            tx,
            session,
            Name::from(&s.name),
            ObjectKind::Graph,
            s.if_exists,
        ),
        S::CreateGraphType(s) => {
            let (parent, name) = target(tx, session, Name::from(&s.name), ObjectKind::GraphType)?;
            if s.or_replace && s.if_not_exists {
                return Err(Error::InvalidDefinition(
                    "OR REPLACE and IF NOT EXISTS are mutually exclusive".into(),
                ));
            }
            if existing(
                tx,
                parent,
                ObjectKind::GraphType,
                &name,
                s.if_not_exists,
                s.or_replace,
            )? {
                return Ok(0);
            }
            let definition = match &s.source {
                ast::CreateGraphTypeSource::Nested(body) => GraphDefinition::bind(body)?,
                ast::CreateGraphTypeSource::CopyOf(reference) => {
                    let id = resolve_type(tx, session, reference)?;
                    type_definition(tx, id)?
                }
                ast::CreateGraphTypeSource::CopyOfExternal(_) => {
                    return Err(Error::UnsupportedFeature(
                        "external graph type imports".into(),
                    ))
                }
                ast::CreateGraphTypeSource::Like(graph) => {
                    match graph_shape_copy(tx, session, graph)? {
                        GraphShape::Inline(definition) => definition,
                        GraphShape::Open => GraphDefinition {
                            open: true,
                            ..GraphDefinition::default()
                        },
                        GraphShape::Named(_) => unreachable!(),
                    }
                }
            };
            if s.or_replace {
                if let Some(old) = tx.lookup(parent, ObjectKind::GraphType, &name) {
                    tx.drop_object(old.id)?;
                }
            }
            tx.create(parent, &name, ObjectDefinition::GraphType(definition))?;
            Ok(1)
        }
        S::DropGraphType(s) => drop_named(
            tx,
            session,
            Name::from(&s.name),
            ObjectKind::GraphType,
            s.if_exists,
        ),
        S::SessionSet(command) => {
            session_set(tx, session, &command.target)?;
            Ok(0)
        }
        S::SessionReset(command) => {
            match &command.target {
                ast::SessionResetTarget::AllCharacteristics => {
                    session.current_schema = initial.current_schema;
                    session.current_graph = initial.current_graph;
                    session.time_zone = initial.time_zone.clone();
                }
                ast::SessionResetTarget::AllParameters => session.parameters.clear(),
                ast::SessionResetTarget::Schema => session.current_schema = initial.current_schema,
                ast::SessionResetTarget::Graph => session.current_graph = initial.current_graph,
                ast::SessionResetTarget::TimeZone => session.time_zone = initial.time_zone.clone(),
                ast::SessionResetTarget::Parameter(name) => {
                    if session.parameters.remove(name).is_none() {
                        return Err(Error::NotFound(format!("parameter {name}")));
                    }
                }
            }
            Ok(0)
        }
        S::SessionClose(_) => {
            session.parameters.clear();
            session.closed = true;
            Ok(0)
        }
        _ => Err(Error::UnsupportedFeature(
            "graph query/data execution or procedure calls".into(),
        )),
    }
}

fn existing(
    tx: &mut StatementTxn,
    parent: ObjectId,
    kind: ObjectKind,
    name: &str,
    if_not_exists: bool,
    replace: bool,
) -> Result<bool> {
    if tx.lookup(parent, kind, name).is_some() {
        if if_not_exists {
            return Ok(true);
        }
        if !replace {
            return Err(Error::AlreadyExists(name.into()));
        }
    }
    Ok(false)
}
fn drop_named(
    tx: &mut StatementTxn,
    session: &SessionState,
    name: Name<'_>,
    kind: ObjectKind,
    if_exists: bool,
) -> Result<usize> {
    let (parent, name) = target(tx, session, name, kind)?;
    match tx.lookup(parent, kind, &name) {
        Some(entry) => {
            tx.drop_object(entry.id)?;
            Ok(1)
        }
        None if if_exists => Ok(0),
        None => Err(Error::NotFound(name)),
    }
}

#[derive(Clone, Copy)]
struct Name<'a> {
    absolute: bool,
    current: bool,
    home: bool,
    parents: usize,
    parts: &'a [ast::Identifier],
}
macro_rules! name_conversion {
    ($($ty:ty),*) => { $(impl<'a> From<&'a $ty> for Name<'a> {
        fn from(n: &'a $ty) -> Self { Self { absolute: n.absolute, current: n.current_schema, home: n.home_schema, parents: n.parent_levels, parts: &n.parts } }
    })* };
}
name_conversion!(ast::GraphName, ast::GraphTypeName, ast::SchemaName);

fn target(
    tx: &mut StatementTxn,
    session: &SessionState,
    name: Name<'_>,
    kind: ObjectKind,
) -> Result<(ObjectId, String)> {
    let Some((last, prefix)) = name.parts.split_last() else {
        return Err(Error::InvalidDefinition("object name required".into()));
    };
    let schema = if name.home {
        session.home_schema
    } else {
        session.current_schema
    };
    if !name.absolute
        && name.parents == 0
        && prefix.is_empty()
        && matches!(kind, ObjectKind::Graph | ObjectKind::GraphType)
    {
        require_kind(tx, schema, ObjectKind::Schema)?;
        return Ok((schema, last.value.clone()));
    }
    let mut directory = if name.absolute {
        ROOT_DIRECTORY
    } else {
        let current = require_kind(tx, schema, ObjectKind::Schema)?;
        if name.parents > 0 {
            let mut id = schema;
            for _ in 0..name.parents {
                id = tx.get(id)?.parent;
                if id == 0 {
                    return Err(Error::InvalidReference(
                        "path escapes root directory".into(),
                    ));
                }
            }
            id
        } else {
            current.parent
        }
    };
    let directories = if matches!(kind, ObjectKind::Graph | ObjectKind::GraphType) {
        if prefix.is_empty() {
            return Err(Error::InvalidReference(
                "qualified graph path requires a schema".into(),
            ));
        }
        &prefix[..prefix.len() - 1]
    } else {
        prefix
    };
    for component in directories {
        directory = tx
            .lookup(directory, ObjectKind::Directory, &component.value)
            .ok_or_else(|| Error::NotFound(format!("directory {}", component.value)))?
            .id;
    }
    if matches!(kind, ObjectKind::Graph | ObjectKind::GraphType) {
        let schema_name = &prefix.last().unwrap().value;
        directory = tx
            .lookup(directory, ObjectKind::Schema, schema_name)
            .ok_or_else(|| Error::NotFound(format!("schema {schema_name}")))?
            .id;
    }
    require_kind(
        tx,
        directory,
        if matches!(kind, ObjectKind::Graph | ObjectKind::GraphType) {
            ObjectKind::Schema
        } else {
            ObjectKind::Directory
        },
    )?;
    Ok((directory, last.value.clone()))
}

pub(crate) fn schema_reference(
    tx: &mut StatementTxn,
    session: &SessionState,
    reference: &ast::SchemaReference,
) -> Result<ObjectId> {
    let id = match reference {
        ast::SchemaReference::CurrentSchema => session.current_schema,
        ast::SchemaReference::HomeSchema => session.home_schema,
        ast::SchemaReference::Root => {
            return Err(Error::InvalidReference(
                "root directory is not a schema".into(),
            ))
        }
        ast::SchemaReference::Parameter(name) => match parameter(session, name)? {
            Value::Schema(id) => *id,
            _ => return Err(Error::InvalidReference("schema parameter required".into())),
        },
        ast::SchemaReference::Name(n) | ast::SchemaReference::Absolute(n) => {
            let mut name = Name::from(n);
            if matches!(reference, ast::SchemaReference::Absolute(_)) {
                name.absolute = true;
            }
            if name.parts.is_empty() && (name.current || name.home) {
                if name.home {
                    session.home_schema
                } else {
                    session.current_schema
                }
            } else {
                let (parent, object) = target(tx, session, name, ObjectKind::Schema)?;
                tx.lookup(parent, ObjectKind::Schema, &object)
                    .ok_or(Error::NotFound(object))?
                    .id
            }
        }
        ast::SchemaReference::Parent { levels, name } => {
            let mut n = Name::from(name);
            n.parents = *levels;
            let (parent, object) = target(tx, session, n, ObjectKind::Schema)?;
            tx.lookup(parent, ObjectKind::Schema, &object)
                .ok_or(Error::NotFound(object))?
                .id
        }
    };
    require_kind(tx, id, ObjectKind::Schema)?;
    Ok(id)
}
fn require_kind(tx: &mut StatementTxn, id: ObjectId, kind: ObjectKind) -> Result<CatalogEntry> {
    let entry = tx.get(id)?;
    if entry.definition.kind() != kind {
        return Err(Error::InvalidReference(format!(
            "expected {kind:?}, found {:?}",
            entry.definition.kind()
        )));
    }
    Ok(entry)
}
pub(crate) fn resolve_graph(
    tx: &mut StatementTxn,
    session: &SessionState,
    expression: &ast::GraphExpression,
) -> Result<ObjectId> {
    let id = match expression {
        ast::GraphExpression::Name(name) => {
            let (parent, object) = target(tx, session, Name::from(name), ObjectKind::Graph)?;
            tx.lookup(parent, ObjectKind::Graph, &object)
                .ok_or(Error::NotFound(object))?
                .id
        }
        ast::GraphExpression::CurrentGraph | ast::GraphExpression::CurrentPropertyGraph => session
            .current_graph
            .ok_or_else(|| Error::InvalidReference("current graph is unset".into()))?,
        ast::GraphExpression::HomeGraph | ast::GraphExpression::HomePropertyGraph => session
            .home_graph
            .ok_or_else(|| Error::InvalidReference("home graph is unset".into()))?,
        ast::GraphExpression::Parameter(name) => match parameter(session, name)? {
            Value::Graph(id) => *id,
            _ => return Err(Error::InvalidReference("graph parameter required".into())),
        },
        ast::GraphExpression::Variable(value) => match eval(tx, session, value)? {
            Value::Graph(id) => id,
            _ => return Err(Error::InvalidReference("graph value required".into())),
        },
    };
    require_kind(tx, id, ObjectKind::Graph)?;
    Ok(id)
}
fn resolve_type(
    tx: &mut StatementTxn,
    session: &SessionState,
    reference: &ast::GraphTypeReference,
) -> Result<ObjectId> {
    let id = match reference {
        ast::GraphTypeReference::Name(name) => {
            let (parent, object) = target(tx, session, Name::from(name), ObjectKind::GraphType)?;
            tx.lookup(parent, ObjectKind::GraphType, &object)
                .ok_or(Error::NotFound(object))?
                .id
        }
        ast::GraphTypeReference::Parameter(name) => match parameter(session, name)? {
            Value::GraphType(id) => *id,
            _ => {
                return Err(Error::InvalidReference(
                    "graph type parameter required".into(),
                ))
            }
        },
    };
    require_kind(tx, id, ObjectKind::GraphType)?;
    Ok(id)
}
fn type_definition(tx: &mut StatementTxn, id: ObjectId) -> Result<GraphDefinition> {
    match require_kind(tx, id, ObjectKind::GraphType)?.definition {
        ObjectDefinition::GraphType(definition) => Ok(definition),
        _ => unreachable!(),
    }
}
fn graph_shape_copy(
    tx: &mut StatementTxn,
    session: &SessionState,
    graph: &ast::GraphExpression,
) -> Result<GraphShape> {
    let id = resolve_graph(tx, session, graph)?;
    match tx.get(id)?.definition {
        ObjectDefinition::Graph {
            shape: GraphShape::Named(id),
            ..
        } => Ok(GraphShape::Inline(type_definition(tx, id)?)),
        ObjectDefinition::Graph { shape, .. } => Ok(shape),
        _ => unreachable!(),
    }
}

fn session_set(
    tx: &mut StatementTxn,
    session: &mut SessionState,
    target: &ast::SessionSetTarget,
) -> Result<()> {
    match target {
        ast::SessionSetTarget::Schema(reference) => {
            session.current_schema = schema_reference(tx, session, reference)?
        }
        ast::SessionSetTarget::Graph(graph) => {
            session.current_graph = Some(resolve_graph(tx, session, graph)?)
        }
        ast::SessionSetTarget::TimeZone(expression) => {
            let Value::String(zone) = eval(tx, session, expression)? else {
                return Err(Error::InvalidDefinition(
                    "time zone must be a string".into(),
                ));
            };
            validate_timezone(&zone)?;
            session.time_zone = zone;
        }
        ast::SessionSetTarget::ValueParameter(p) => {
            if p.if_not_exists && session.parameters.contains_key(&p.name) {
                return Ok(());
            }
            let value = eval(tx, session, &p.initializer)?;
            install_parameter(tx, session, &p.name, value, p.value_type.as_ref())?;
        }
        ast::SessionSetTarget::GraphParameter(p) => {
            if p.if_not_exists && session.parameters.contains_key(&p.name) {
                return Ok(());
            }
            let value = Value::Graph(resolve_graph(tx, session, &p.initializer)?);
            install_parameter(tx, session, &p.name, value, p.value_type.as_ref())?;
        }
        ast::SessionSetTarget::BindingTableParameter(p) => {
            if p.if_not_exists && session.parameters.contains_key(&p.name) {
                return Ok(());
            }
            let value = match &p.initializer {
                ast::BindingTableExpression::Variable(expression) => eval(tx, session, expression)?,
                ast::BindingTableExpression::Name(name)
                    if name.parts.len() == 1
                        && !name.absolute
                        && !name.current_schema
                        && !name.home_schema
                        && name.parent_levels == 0 =>
                {
                    parameter(session, &name.parts[0].value)?.clone()
                }
                _ => {
                    return Err(Error::UnsupportedFeature(
                        "binding-table query or catalog initializer".into(),
                    ))
                }
            };
            if !matches!(value, Value::BindingTable(_)) {
                return Err(Error::InvalidDefinition("binding table required".into()));
            }
            install_parameter(tx, session, &p.name, value, p.value_type.as_ref())?;
        }
    }
    Ok(())
}
fn install_parameter(
    tx: &mut StatementTxn,
    session: &mut SessionState,
    name: &str,
    value: Value,
    declaration: Option<&ast::ValueType>,
) -> Result<()> {
    validate_references(tx, &value)?;
    let declared_type = declaration.map(ValueType::bind).transpose()?;
    if let Some(t) = &declared_type {
        check_value(tx, &value, t)?;
    }
    session.parameters.insert(
        name.into(),
        Parameter {
            value,
            declared_type,
        },
    );
    Ok(())
}
fn parameter<'a>(session: &'a SessionState, name: &str) -> Result<&'a Value> {
    session
        .parameters
        .get(name)
        .map(|p| &p.value)
        .ok_or_else(|| Error::NotFound(format!("parameter {name}")))
}

fn eval(tx: &mut StatementTxn, session: &SessionState, expression: &ast::Expr) -> Result<Value> {
    let value = match expression {
        ast::Expr::Parameter(name) | ast::Expr::Identifier(ast::Identifier { value: name }) => {
            parameter(session, name)?.clone()
        }
        ast::Expr::GraphReference { graph, .. } => Value::Graph(resolve_graph(tx, session, graph)?),
        ast::Expr::Literal(literal) => match literal {
            ast::Literal::Null | ast::Literal::Unknown => Value::Null,
            ast::Literal::Boolean(v) => Value::Boolean(*v),
            ast::Literal::Integer(v) => Value::Integer(*v),
            ast::Literal::Decimal(v) => Value::Float(*v),
            ast::Literal::String(v) => Value::String(v.clone()),
            ast::Literal::Bytes(v) => Value::Bytes(v.clone()),
            _ => {
                return Err(Error::UnsupportedFeature(
                    "temporal parameter literal evaluation".into(),
                ))
            }
        },
        ast::Expr::List(items) | ast::Expr::TypedList { values: items, .. } => Value::List(
            items
                .iter()
                .map(|e| eval(tx, session, e))
                .collect::<Result<_>>()?,
        ),
        ast::Expr::Record { fields, .. } | ast::Expr::Map(fields) => {
            let mut values = BTreeMap::new();
            for (key, expression) in &fields.entries {
                if values
                    .insert(key.value.clone(), eval(tx, session, expression)?)
                    .is_some()
                {
                    return Err(Error::InvalidDefinition(format!(
                        "duplicate record field {}",
                        key.value
                    )));
                }
            }
            Value::Record(values)
        }
        ast::Expr::Unary { op, expr } => match (op, eval(tx, session, expr)?) {
            (ast::UnaryOp::Pos, value @ (Value::Integer(_) | Value::Float(_))) => value,
            (ast::UnaryOp::Neg, Value::Integer(value)) => Value::Integer(
                value
                    .checked_neg()
                    .ok_or_else(|| Error::InvalidDefinition("integer overflow".into()))?,
            ),
            (ast::UnaryOp::Neg, Value::Float(value)) => Value::Float(-value),
            (ast::UnaryOp::Not, Value::Boolean(value)) => Value::Boolean(!value),
            (ast::UnaryOp::Not, Value::Null) => Value::Null,
            _ => return Err(Error::InvalidDefinition("invalid unary operand".into())),
        },
        _ => {
            return Err(Error::UnsupportedFeature(
                "parameter initializer expression".into(),
            ))
        }
    };
    validate_references(tx, &value)?;
    Ok(value)
}
fn validate_references(tx: &mut StatementTxn, value: &Value) -> Result<()> {
    match value {
        Value::Graph(id) => {
            require_kind(tx, *id, ObjectKind::Graph)?;
        }
        Value::GraphType(id) => {
            require_kind(tx, *id, ObjectKind::GraphType)?;
        }
        Value::Schema(id) => {
            require_kind(tx, *id, ObjectKind::Schema)?;
        }
        Value::List(values) => {
            for value in values {
                validate_references(tx, value)?;
            }
        }
        Value::Record(values) => {
            for value in values.values() {
                validate_references(tx, value)?;
            }
        }
        Value::BindingTable(rows) => {
            for row in rows {
                for value in row.values() {
                    validate_references(tx, value)?;
                }
            }
        }
        _ => (),
    }
    Ok(())
}
fn check_value(tx: &mut StatementTxn, value: &Value, t: &ValueType) -> Result<()> {
    if matches!(value, Value::Null) {
        return if t.nullable {
            Ok(())
        } else {
            Err(Error::InvalidDefinition("NULL violates NOT NULL".into()))
        };
    }
    let valid = match (t.kind.as_str(), value) {
        ("any", _)
        | ("boolean", Value::Boolean(_))
        | ("float", Value::Float(_) | Value::Integer(_)) => true,
        ("integer", Value::Integer(v)) => {
            let bits = t.parameters.get("precision").copied().unwrap_or(64);
            if t.parameters.get("signed") == Some(&0) {
                *v >= 0 && (bits >= 63 || (*v as u64) < (1u64 << bits))
            } else {
                bits >= 64
                    || (bits > 0
                        && (*v as i128) >= -(1i128 << (bits - 1))
                        && (*v as i128) < (1i128 << (bits - 1)))
            }
        }
        ("graph_ref", Value::Graph(id)) => {
            if let Some(required) = &t.graph {
                match require_kind(tx, *id, ObjectKind::Graph)?.definition {
                    ObjectDefinition::Graph {
                        shape: GraphShape::Inline(actual),
                        ..
                    } => actual == **required,
                    ObjectDefinition::Graph {
                        shape: GraphShape::Named(id),
                        ..
                    } => type_definition(tx, id)? == **required,
                    _ => false,
                }
            } else {
                true
            }
        }
        ("string", Value::String(v)) => t
            .parameters
            .get("max_length")
            .is_none_or(|n| v.chars().count() as u64 <= *n),
        ("bytes", Value::Bytes(v)) => {
            t.parameters
                .get("min_length")
                .is_none_or(|n| v.len() as u64 >= *n)
                && t.parameters
                    .get("max_length")
                    .is_none_or(|n| v.len() as u64 <= *n)
        }
        ("list" | "array", Value::List(items)) => {
            t.parameters
                .get("max_length")
                .is_none_or(|n| items.len() as u64 <= *n)
                && items
                    .iter()
                    .all(|v| check_value(tx, v, &t.arguments[0]).is_ok())
        }
        ("record", Value::Record(fields)) => {
            fields.len() == t.fields.len()
                && t.fields.iter().all(|(k, ty)| {
                    fields
                        .get(k)
                        .is_some_and(|v| check_value(tx, v, ty).is_ok())
                })
        }
        ("open_record", Value::Record(_)) => true,
        ("binding_table", Value::BindingTable(rows)) => rows.iter().all(|row| {
            row.len() == t.fields.len()
                && t.fields
                    .iter()
                    .all(|(k, ty)| row.get(k).is_some_and(|v| check_value(tx, v, ty).is_ok()))
        }),
        ("union", _) => t
            .arguments
            .iter()
            .any(|ty| check_value(tx, value, ty).is_ok()),
        (
            "property_value",
            Value::Boolean(_)
            | Value::Integer(_)
            | Value::Float(_)
            | Value::String(_)
            | Value::Bytes(_),
        ) => true,
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(Error::InvalidDefinition(format!(
            "value does not satisfy {}",
            t.kind
        )))
    }
}
fn validate_timezone(zone: &str) -> Result<()> {
    if zone == "UTC" || zone == "Z" {
        return Ok(());
    }
    let bytes = zone.as_bytes();
    if bytes.len() == 6
        && matches!(bytes[0], b'+' | b'-')
        && bytes[3] == b':'
        && [bytes[1], bytes[2], bytes[4], bytes[5]]
            .iter()
            .all(u8::is_ascii_digit)
    {
        let hours = (bytes[1] - b'0') * 10 + bytes[2] - b'0';
        let minutes = (bytes[4] - b'0') * 10 + bytes[5] - b'0';
        if hours <= 14 && minutes < 60 && (hours != 14 || minutes == 0) {
            return Ok(());
        }
    }
    Err(Error::UnsupportedFeature("time zone must be UTC or a numeric offset (+08:00); named zone evaluation is not implemented".into()))
}
