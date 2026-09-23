//! Bound, versionable type definitions independent of parser AST layout.
use crate::{gql as ast, Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValueType {
    pub kind: String,
    pub nullable: bool,
    pub parameters: BTreeMap<String, u64>,
    pub arguments: Vec<ValueType>,
    pub fields: BTreeMap<String, ValueType>,
    pub graph: Option<Box<GraphDefinition>>,
    pub edge: Option<Box<EdgeType>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeType {
    pub labels: BTreeSet<String>,
    pub properties: BTreeMap<String, ValueType>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeType {
    pub labels: BTreeSet<String>,
    pub properties: BTreeMap<String, ValueType>,
    pub direction: EdgeDirection,
    pub source: Option<String>,
    pub destination: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EdgeDirection {
    Left,
    Right,
    Undirected,
    LeftOrUndirected,
    UndirectedOrRight,
    LeftOrRight,
    Any,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphDefinition {
    pub open: bool,
    pub nodes: BTreeMap<String, NodeType>,
    pub edges: BTreeMap<String, EdgeType>,
}

impl GraphDefinition {
    pub(crate) fn bind(body: &ast::GraphTypeBody) -> Result<Self> {
        let mut result = Self::default();
        for element in &body.elements {
            match element {
                ast::GraphTypeElement::Node(n) => {
                    if result
                        .nodes
                        .insert(
                            n.name.value.clone(),
                            NodeType {
                                labels: labels(&n.labels)?,
                                properties: properties(&n.properties)?,
                            },
                        )
                        .is_some()
                    {
                        return duplicate(&n.name.value);
                    }
                }
                ast::GraphTypeElement::Edge(e) => {
                    let direction = edge_direction(e.direction);
                    if result
                        .edges
                        .insert(
                            e.name.value.clone(),
                            EdgeType {
                                labels: labels(&e.labels)?,
                                properties: properties(&e.properties)?,
                                direction,
                                source: e.source.as_ref().map(|n| n.value.clone()),
                                destination: e.destination.as_ref().map(|n| n.value.clone()),
                            },
                        )
                        .is_some()
                    {
                        return duplicate(&e.name.value);
                    }
                }
            }
        }
        for (name, edge) in &result.edges {
            if result.nodes.contains_key(name) {
                return duplicate(name);
            }
            for endpoint in [&edge.source, &edge.destination].into_iter().flatten() {
                if !result.nodes.contains_key(endpoint) {
                    return Err(Error::InvalidDefinition(format!(
                        "unknown edge endpoint {endpoint}"
                    )));
                }
            }
        }
        Ok(result)
    }
}

impl ValueType {
    fn scalar(kind: &str) -> Self {
        Self {
            kind: kind.into(),
            nullable: true,
            parameters: BTreeMap::new(),
            arguments: Vec::new(),
            fields: BTreeMap::new(),
            graph: None,
            edge: None,
        }
    }
    pub(crate) fn bind(value: &ast::ValueType) -> Result<Self> {
        use ast::ValueType as T;
        let mut result = match value {
            T::NotNull(inner) => {
                let mut t = Self::bind(inner)?;
                t.nullable = false;
                return Ok(t);
            }
            T::Named { name, parameters } => {
                let kind = match name.value.to_ascii_uppercase().as_str() {
                    "INT" | "INTEGER" | "BIGINT" | "SMALLINT" | "INT64" | "INT32" | "INT16"
                    | "INT8" | "UINT64" | "UINT32" | "UINT16" | "UINT8" => "integer",
                    "STRING" | "VARCHAR" | "CHAR" => "string",
                    "BOOL" | "BOOLEAN" => "boolean",
                    "FLOAT" | "REAL" | "DOUBLE" => "float",
                    "DECIMAL" | "DEC" | "NUMERIC" => "decimal",
                    _ => {
                        return Err(Error::UnsupportedFeature(format!(
                            "named value type {}",
                            name.value
                        )))
                    }
                };
                let mut t = Self::scalar(kind);
                // Named syntax is retained semantically; precision is not an AST serialization.
                for (i, v) in parameters.iter().enumerate() {
                    t.parameters.insert(format!("parameter_{i}"), *v);
                }
                t
            }
            T::CharacterString { max_length, .. } => {
                let mut t = Self::scalar("string");
                t.optional("max_length", *max_length);
                t
            }
            T::Boolean { .. } => Self::scalar("boolean"),
            T::Temporal { kind } => Self::scalar(match kind {
                ast::TemporalTypeKind::ZonedDateTime => "zoned_datetime",
                ast::TemporalTypeKind::LocalDateTime => "local_datetime",
                ast::TemporalTypeKind::Date => "date",
                ast::TemporalTypeKind::ZonedTime => "zoned_time",
                ast::TemporalTypeKind::LocalTime => "local_time",
                ast::TemporalTypeKind::Duration => "duration",
            }),
            T::ByteString {
                min_length,
                max_length,
                ..
            } => {
                let mut t = Self::scalar("bytes");
                t.optional("min_length", *min_length);
                t.optional("max_length", *max_length);
                t
            }
            T::ExactNumeric {
                kind,
                signed,
                precision,
                scale,
            } => {
                let mut t = Self::scalar(match kind {
                    ast::ExactNumericTypeKind::Integer => "integer",
                    ast::ExactNumericTypeKind::Decimal => "decimal",
                });
                t.optional("signed", signed.map(u64::from));
                t.optional("precision", *precision);
                t.optional("scale", *scale);
                t
            }
            T::ApproximateNumeric { precision, .. } => {
                let mut t = Self::scalar("float");
                t.optional("precision", *precision);
                t
            }
            T::List(inner)
            | T::Array(inner)
            | T::ListWithLength { element: inner, .. }
            | T::ArrayWithLength { element: inner, .. } => {
                let mut t = Self::scalar(
                    if matches!(value, T::Array(_) | T::ArrayWithLength { .. }) {
                        "array"
                    } else {
                        "list"
                    },
                );
                t.arguments.push(Self::bind(inner)?);
                match value {
                    T::ListWithLength { max_length, .. }
                    | T::ArrayWithLength { max_length, .. } => {
                        t.parameters.insert("max_length".into(), *max_length)
                    }
                    _ => None,
                };
                t
            }
            T::Record(fields) | T::BindingTable { fields, .. } => {
                let mut t = Self::scalar(if matches!(value, T::Record(_)) {
                    "record"
                } else {
                    "binding_table"
                });
                for field in fields {
                    if t.fields
                        .insert(field.name.value.clone(), Self::bind(&field.value_type)?)
                        .is_some()
                    {
                        return duplicate(&field.name.value);
                    }
                }
                t
            }
            T::DynamicUnion(types) => {
                let mut t = Self::scalar("union");
                t.arguments = types.iter().map(Self::bind).collect::<Result<_>>()?;
                t
            }
            T::GraphReference { body, .. } => {
                let mut t = Self::scalar("graph_ref");
                t.graph = body
                    .as_ref()
                    .map(GraphDefinition::bind)
                    .transpose()?
                    .map(Box::new);
                t
            }
            T::NodeReference { definition } => {
                let mut t = Self::scalar("node_ref");
                t.graph = definition
                    .as_ref()
                    .map(|n| {
                        GraphDefinition::bind(&ast::GraphTypeBody {
                            elements: vec![ast::GraphTypeElement::Node(n.clone())],
                        })
                    })
                    .transpose()?
                    .map(Box::new);
                t
            }
            T::EdgeReference { definition: None } => Self::scalar("edge_ref"),
            T::EdgeReference {
                definition: Some(e),
            } => {
                let mut t = Self::scalar("edge_ref");
                t.edge = Some(Box::new(EdgeType {
                    labels: labels(&e.labels)?,
                    properties: properties(&e.properties)?,
                    direction: edge_direction(e.direction),
                    source: e.source.as_ref().map(|n| n.value.clone()),
                    destination: e.destination.as_ref().map(|n| n.value.clone()),
                }));
                t
            }
            T::AnyRecord => Self::scalar("open_record"),
            T::AnyDynamic => Self::scalar("any"),
            T::PropertyValue => Self::scalar("property_value"),
            T::Path => Self::scalar("path"),
        };
        if let (Some(min), Some(max)) = (
            result.parameters.get("min_length"),
            result.parameters.get("max_length"),
        ) {
            if min > max {
                return Err(Error::InvalidDefinition(
                    "minimum length exceeds maximum".into(),
                ));
            }
        }
        if result.parameters.get("precision") == Some(&0) {
            return Err(Error::InvalidDefinition(
                "precision must be positive".into(),
            ));
        }
        if let (Some(scale), Some(precision)) = (
            result.parameters.get("scale"),
            result.parameters.get("precision"),
        ) {
            if scale > precision {
                return Err(Error::InvalidDefinition("scale exceeds precision".into()));
            }
        }
        result.arguments.shrink_to_fit();
        Ok(result)
    }
    fn optional(&mut self, key: &str, value: Option<u64>) {
        if let Some(v) = value {
            self.parameters.insert(key.into(), v);
        }
    }
}

fn labels(values: &[ast::Identifier]) -> Result<BTreeSet<String>> {
    let mut result = BTreeSet::new();
    for value in values {
        if !result.insert(value.value.clone()) {
            return duplicate(&value.value);
        }
    }
    Ok(result)
}
fn properties(values: &[ast::PropertyTypeDefinition]) -> Result<BTreeMap<String, ValueType>> {
    let mut result = BTreeMap::new();
    for value in values {
        if result
            .insert(
                value.name.value.clone(),
                ValueType::bind(&value.value_type)?,
            )
            .is_some()
        {
            return duplicate(&value.name.value);
        }
    }
    Ok(result)
}
fn duplicate<T>(name: &str) -> Result<T> {
    Err(Error::InvalidDefinition(format!(
        "duplicate definition: {name}"
    )))
}

fn edge_direction(direction: ast::Direction) -> EdgeDirection {
    match direction {
        ast::Direction::Left => EdgeDirection::Left,
        ast::Direction::Right => EdgeDirection::Right,
        ast::Direction::Undirected => EdgeDirection::Undirected,
        ast::Direction::LeftOrUndirected => EdgeDirection::LeftOrUndirected,
        ast::Direction::UndirectedOrRight => EdgeDirection::UndirectedOrRight,
        ast::Direction::LeftOrRight => EdgeDirection::LeftOrRight,
        ast::Direction::Any => EdgeDirection::Any,
    }
}
