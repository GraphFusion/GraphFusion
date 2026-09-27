//! Read-only, depth-first traversal of the complete owned AST.
//!
//! Override a typed `visit_*` method to inspect a node, and call its matching
//! `walk_*` function to recurse. Omitting that call skips the subtree. Fields
//! follow declaration order, and lists follow stored order; this is structural
//! order, not query execution order. In particular, compatibility fields such
//! as a path's `start`/`chains` and `factors` are all visited, even when they
//! describe the same syntax.
//!
//! Returning [`ControlFlow::Break`] stops traversal immediately. Exit hooks are
//! called only for successfully completed nodes, fields, and lists, so they
//! must not be relied on for cleanup after a break.
//!
//! ```
//! use std::ops::ControlFlow;
//! use gql_parser::{parse, Expr};
//! use gql_parser::visit::{self, Visitor};
//!
//! struct Parameters<'ast>(Vec<&'ast str>);
//! impl<'ast> Visitor<'ast> for Parameters<'ast> {
//!     type Break = std::convert::Infallible;
//!
//!     fn visit_expr(&mut self, expr: &'ast Expr) -> ControlFlow<Self::Break> {
//!         if let Expr::Parameter(name) = expr {
//!             self.0.push(name);
//!         }
//!         visit::walk_expr(self, expr)
//!     }
//! }
//!
//! let ast = parse("RETURN $name AS value").unwrap();
//! let mut parameters = Parameters(Vec::new());
//! let ControlFlow::Continue(()) = parameters.visit_program(&ast);
//! assert_eq!(parameters.0, ["name"]);
//! ```

use std::fmt;
use std::ops::ControlFlow;

use crate::ast::*;

/// The edge from a node to a named field, or from a list to one of its elements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Named(&'static str),
    Index(usize),
}

impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Named(name) => f.write_str(name),
            Self::Index(index) => write!(f, "[{index}]"),
        }
    }
}

/// A leaf value. `None` is explicit; `Some` and `Box` are transparent.
///
/// A missing optional list therefore differs from a present, empty list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scalar<'ast> {
    None,
    Bool(bool),
    Unsigned(u64),
    Signed(i64),
    Float(f64),
    String(&'ast str),
}

impl fmt::Display for Scalar<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => f.write_str("None"),
            Self::Bool(value) => write!(f, "{value}"),
            Self::Unsigned(value) => write!(f, "{value}"),
            Self::Signed(value) => write!(f, "{value}"),
            Self::Float(value) => write!(f, "{value:?}"),
            Self::String(value) => write!(f, "{value:?}"),
        }
    }
}

// The schema below lists every field without `..` and every enum variant without
// a catch-all. Changing an AST shape requires updating the corresponding walker.
// It also generates typed dispatch and node metadata, keeping printing and other
// visitors on exactly the same traversal.
macro_rules! define_visitor {
    (
        structs {
            $($struct:ident => $visit_struct:ident, $walk_struct:ident { $($field:ident),* $(,)? })*
        }
        enums {
            $($enum:ident => $visit_enum:ident, $walk_enum:ident {
                $($variant:ident $(($($tuple:ident),+))? $({$($named:ident),+})?),* $(,)?
            })*
        }
    ) => {
        /// A borrowed, typed AST node passed to the generic entry/exit hooks.
        #[derive(Clone, Copy, Debug)]
        pub enum AstNode<'ast> {
            $($struct(&'ast $struct),)*
            $($enum(&'ast $enum),)*
        }

        impl AstNode<'_> {
            /// The Rust AST type, qualified by its variant for enum nodes.
            pub fn kind(self) -> &'static str {
                match self {
                    $(Self::$struct(_) => stringify!($struct),)*
                    $(Self::$enum(node) => match node {
                        $($enum::$variant $(($($tuple),+))? $({$($named),+})? => {
                            $($(let _ = $tuple;)+)?
                            $($(let _ = $named;)+)?
                            concat!(stringify!($enum), "::", stringify!($variant))
                        },)*
                    },)*
                }
            }
        }

        /// A read-only AST visitor with optional early termination.
        ///
        /// Typed methods recurse by default. Generic hooks observe the same
        /// traversal and are useful for renderers and structural tooling.
        /// Implementations may borrow nodes for the AST's entire lifetime.
        pub trait Visitor<'ast> {
            type Break;

            fn enter_node(&mut self, _node: AstNode<'ast>) -> ControlFlow<Self::Break> {
                ControlFlow::Continue(())
            }

            fn leave_node(&mut self, _node: AstNode<'ast>) -> ControlFlow<Self::Break> {
                ControlFlow::Continue(())
            }

            fn enter_field(&mut self, _field: Field) -> ControlFlow<Self::Break> {
                ControlFlow::Continue(())
            }

            fn leave_field(&mut self, _field: Field) -> ControlFlow<Self::Break> {
                ControlFlow::Continue(())
            }

            fn enter_list(&mut self, _len: usize) -> ControlFlow<Self::Break> {
                ControlFlow::Continue(())
            }

            fn leave_list(&mut self) -> ControlFlow<Self::Break> {
                ControlFlow::Continue(())
            }

            fn visit_scalar(&mut self, _value: Scalar<'ast>) -> ControlFlow<Self::Break> {
                ControlFlow::Continue(())
            }

            $(fn $visit_struct(&mut self, node: &'ast $struct) -> ControlFlow<Self::Break> {
                $walk_struct(self, node)
            })*

            $(fn $visit_enum(&mut self, node: &'ast $enum) -> ControlFlow<Self::Break> {
                $walk_enum(self, node)
            })*
        }

        $(
            #[doc = concat!("Walk every field of [`", stringify!($struct), "`] in declaration order.")]
            pub fn $walk_struct<'ast, V: Visitor<'ast> + ?Sized>(
                visitor: &mut V,
                node: &'ast $struct,
            ) -> ControlFlow<V::Break> {
                let $struct { $($field),* } = node;
                visitor.enter_node(AstNode::$struct(node))?;
                $(walk_field(visitor, Field::Named(stringify!($field)), $field)?;)*
                visitor.leave_node(AstNode::$struct(node))
            }

            impl Walk for $struct {
                fn walk<'ast, V: Visitor<'ast> + ?Sized>(&'ast self, visitor: &mut V) -> ControlFlow<V::Break> {
                    visitor.$visit_struct(self)
                }
            }
        )*

        $(
            #[doc = concat!("Walk every field of the active [`", stringify!($enum), "`] variant.")]
            pub fn $walk_enum<'ast, V: Visitor<'ast> + ?Sized>(
                visitor: &mut V,
                node: &'ast $enum,
            ) -> ControlFlow<V::Break> {
                visitor.enter_node(AstNode::$enum(node))?;
                match node {
                    $($enum::$variant $(($($tuple),+))? $({$($named),+})? => {
                        $($(walk_field(visitor, Field::Named(stringify!($tuple)), $tuple)?;)+)?
                        $($(walk_field(visitor, Field::Named(stringify!($named)), $named)?;)+)?
                    },)*
                }
                visitor.leave_node(AstNode::$enum(node))
            }

            impl Walk for $enum {
                fn walk<'ast, V: Visitor<'ast> + ?Sized>(&'ast self, visitor: &mut V) -> ControlFlow<V::Break> {
                    visitor.$visit_enum(self)
                }
            }
        )*
    };
}

trait Walk {
    fn walk<'ast, V: Visitor<'ast> + ?Sized>(&'ast self, visitor: &mut V) -> ControlFlow<V::Break>;
}

fn walk_field<'ast, V: Visitor<'ast> + ?Sized, T: Walk>(
    visitor: &mut V,
    field: Field,
    value: &'ast T,
) -> ControlFlow<V::Break> {
    visitor.enter_field(field)?;
    value.walk(visitor)?;
    visitor.leave_field(field)
}

impl<T: Walk> Walk for Option<T> {
    fn walk<'ast, V: Visitor<'ast> + ?Sized>(&'ast self, visitor: &mut V) -> ControlFlow<V::Break> {
        match self {
            Some(value) => value.walk(visitor),
            None => visitor.visit_scalar(Scalar::None),
        }
    }
}

impl<T: Walk> Walk for Box<T> {
    fn walk<'ast, V: Visitor<'ast> + ?Sized>(&'ast self, visitor: &mut V) -> ControlFlow<V::Break> {
        self.as_ref().walk(visitor)
    }
}

impl<T: Walk> Walk for Vec<T> {
    fn walk<'ast, V: Visitor<'ast> + ?Sized>(&'ast self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.enter_list(self.len())?;
        for (index, value) in self.iter().enumerate() {
            walk_field(visitor, Field::Index(index), value)?;
        }
        visitor.leave_list()
    }
}

impl Walk for (Identifier, Expr) {
    fn walk<'ast, V: Visitor<'ast> + ?Sized>(&'ast self, visitor: &mut V) -> ControlFlow<V::Break> {
        walk_field(visitor, Field::Named("key"), &self.0)?;
        walk_field(visitor, Field::Named("value"), &self.1)
    }
}

impl Walk for String {
    fn walk<'ast, V: Visitor<'ast> + ?Sized>(&'ast self, visitor: &mut V) -> ControlFlow<V::Break> {
        visitor.visit_scalar(Scalar::String(self))
    }
}

macro_rules! scalar_walk {
    ($($type:ty => $variant:ident $(as $cast:ty)?),* $(,)?) => {
        $(impl Walk for $type {
            fn walk<'ast, V: Visitor<'ast> + ?Sized>(&'ast self, visitor: &mut V) -> ControlFlow<V::Break> {
                visitor.visit_scalar(Scalar::$variant(*self $(as $cast)?))
            }
        })*
    };
}

scalar_walk! {
    bool => Bool,
    u8 => Unsigned as u64,
    u64 => Unsigned,
    usize => Unsigned as u64,
    i64 => Signed,
    f64 => Float,
}

define_visitor! {
    structs {
        Program => visit_program, walk_program { at_schema, definitions, statements }
        GraphVariableDefinition => visit_graph_variable_definition, walk_graph_variable_definition { property_graph, name, typed, value_type, initializer }
        BindingTableVariableDefinition => visit_binding_table_variable_definition, walk_binding_table_variable_definition { binding, name, typed, value_type, initializer }
        ValueVariableDefinition => visit_value_variable_definition, walk_value_variable_definition { name, typed, value_type, initializer }
        NextStatement => visit_next_statement, walk_next_statement { yield_clause, statement }
        QueryStatement => visit_query_statement, walk_query_statement { at_schema, use_graph, body, set_operations }
        QueryBody => visit_query_body, walk_query_body { clauses, result_clause, select_from, select_query, select_where, group_by, empty_grouping_set, having, order_by, offset, limit }
        QuerySetOperation => visit_query_set_operation, walk_query_set_operation { operator, quantifier, body }
        SelectGraphMatch => visit_select_graph_match, walk_select_graph_match { graph, match_clause }
        SelectQuerySpecification => visit_select_query_specification, walk_select_query_specification { graph, query }
        MatchClause => visit_match_clause, walk_match_clause { optional, mode, patterns, keep, where_clause, yield_clause }
        FilterClause => visit_filter_clause, walk_filter_clause { predicate }
        LetClause => visit_let_clause, walk_let_clause { items }
        LetItem => visit_let_item, walk_let_item { name, typed, value_type, value }
        ForClause => visit_for_clause, walk_for_clause { variable, source, ordinality_or_offset }
        ForOrdinalityOrOffset => visit_for_ordinality_or_offset, walk_for_ordinality_or_offset { kind, variable }
        CallClause => visit_call_clause, walk_call_clause { optional, call, yield_clause }
        InlineProcedureCall => visit_inline_procedure_call, walk_inline_procedure_call { variable_scope, body }
        OrderByPageClause => visit_order_by_page_clause, walk_order_by_page_clause { order_by, offset, limit }
        CallStatement => visit_call_statement, walk_call_statement { call }
        YieldClause => visit_yield_clause, walk_yield_clause { items }
        ResultClause => visit_result_clause, walk_result_clause { kind, quantifier, distinct, items }
        ResultItem => visit_result_item, walk_result_item { expr, alias }
        OrderByItem => visit_order_by_item, walk_order_by_item { expr, direction, null_ordering }
        InsertStatement => visit_insert_statement, walk_insert_statement { patterns }
        DeleteStatement => visit_delete_statement, walk_delete_statement { detach, items }
        SetStatement => visit_set_statement, walk_set_statement { items }
        RemoveStatement => visit_remove_statement, walk_remove_statement { items }
        SessionSetCommand => visit_session_set_command, walk_session_set_command { target }
        SessionSetGraphParameter => visit_session_set_graph_parameter, walk_session_set_graph_parameter { property_graph, if_not_exists, name, typed, value_type, initializer }
        SessionSetBindingTableParameter => visit_session_set_binding_table_parameter, walk_session_set_binding_table_parameter { binding, if_not_exists, name, typed, value_type, initializer }
        SessionSetValueParameter => visit_session_set_value_parameter, walk_session_set_value_parameter { if_not_exists, name, typed, value_type, initializer }
        SessionResetCommand => visit_session_reset_command, walk_session_reset_command { target }
        SessionCloseCommand => visit_session_close_command, walk_session_close_command {  }
        CreateGraphStatement => visit_create_graph_statement, walk_create_graph_statement { if_not_exists, or_replace, property_graph, name, graph_type, source }
        DropGraphStatement => visit_drop_graph_statement, walk_drop_graph_statement { if_exists, property_graph, name }
        CreateGraphTypeStatement => visit_create_graph_type_statement, walk_create_graph_type_statement { if_not_exists, or_replace, property_graph, name, source }
        DropGraphTypeStatement => visit_drop_graph_type_statement, walk_drop_graph_type_statement { if_exists, property_graph, name }
        CreateSchemaStatement => visit_create_schema_statement, walk_create_schema_statement { if_not_exists, name }
        DropSchemaStatement => visit_drop_schema_statement, walk_drop_schema_statement { if_exists, name }
        StartTransactionStatement => visit_start_transaction_statement, walk_start_transaction_statement { access_mode, isolation_level }
        CommitStatement => visit_commit_statement, walk_commit_statement {  }
        RollbackStatement => visit_rollback_statement, walk_rollback_statement {  }
        GraphName => visit_graph_name, walk_graph_name { absolute, current_schema, home_schema, parent_levels, parts }
        BindingTableName => visit_binding_table_name, walk_binding_table_name { absolute, current_schema, home_schema, parent_levels, parts }
        GraphTypeName => visit_graph_type_name, walk_graph_type_name { absolute, current_schema, home_schema, parent_levels, parts }
        SchemaName => visit_schema_name, walk_schema_name { absolute, current_schema, home_schema, parent_levels, parts }
        ProcedureName => visit_procedure_name, walk_procedure_name { absolute, current_schema, home_schema, parent_levels, parts }
        PathPattern => visit_path_pattern, walk_path_pattern { variable, prefix, parenthesized, start, chains, factors, alternation, alternatives }
        ParenthesizedPathPatternExpression => visit_parenthesized_path_pattern_expression, walk_parenthesized_path_pattern_expression { variable, prefix, pattern, where_clause, quantifier, questioned }
        PathPatternTerm => visit_path_pattern_term, walk_path_pattern_term { start, chains, factors }
        PathPatternChain => visit_path_pattern_chain, walk_path_pattern_chain { relationship, node }
        NodePattern => visit_node_pattern, walk_node_pattern { variable, temporary, labels, label_expression, properties, where_clause, quantifier }
        RelationshipPattern => visit_relationship_pattern, walk_relationship_pattern { direction, variable, temporary, labels, label_expression, properties, where_clause, quantifier }
        GraphTypeBody => visit_graph_type_body, walk_graph_type_body { elements }
        NodeTypeDefinition => visit_node_type_definition, walk_node_type_definition { name, labels, properties }
        EdgeTypeDefinition => visit_edge_type_definition, walk_edge_type_definition { name, direction, labels, source, destination, properties }
        PropertyTypeDefinition => visit_property_type_definition, walk_property_type_definition { name, value_type }
        FieldType => visit_field_type, walk_field_type { name, value_type }
        CaseWhenClause => visit_case_when_clause, walk_case_when_clause { condition, additional_operands, result }
        MapLiteral => visit_map_literal, walk_map_literal { entries }
        SqlIntervalQualifier => visit_sql_interval_qualifier, walk_sql_interval_qualifier { start, start_precision, end, end_precision }
        Identifier => visit_identifier, walk_identifier { value }
    }
    enums {
        BindingVariableDefinition => visit_binding_variable_definition, walk_binding_variable_definition {
            Graph(value),
            BindingTable(value),
            Value(value),
        }
        Statement => visit_statement, walk_statement {
            LinearCatalog(value),
            Query(value),
            Call(value),
            Next(value),
            Insert(value),
            Delete(value),
            Set(value),
            Remove(value),
            CreateGraph(value),
            DropGraph(value),
            CreateGraphType(value),
            DropGraphType(value),
            CreateSchema(value),
            DropSchema(value),
            SessionSet(value),
            SessionReset(value),
            SessionClose(value),
            StartTransaction(value),
            Commit(value),
            Rollback(value),
        }
        QuerySetOperator => visit_query_set_operator, walk_query_set_operator {
            Union,
            Except,
            Intersect,
            Otherwise,
        }
        SetQuantifier => visit_set_quantifier, walk_set_quantifier {
            Distinct,
            All,
        }
        QueryClause => visit_query_clause, walk_query_clause {
            UseGraph(value),
            NestedQuery(value),
            Match(value),
            OptionalMatchBlock(value),
            Filter(value),
            Let(value),
            For(value),
            Call(value),
            Insert(value),
            Delete(value),
            Set(value),
            Remove(value),
            OrderByPage(value),
        }
        MatchMode => visit_match_mode, walk_match_mode {
            RepeatableElements { bindings },
            DifferentEdges { bindings },
        }
        ForOrdinalityOrOffsetKind => visit_for_ordinality_or_offset_kind, walk_for_ordinality_or_offset_kind {
            Ordinality,
            Offset,
        }
        ProcedureCall => visit_procedure_call, walk_procedure_call {
            Named { procedure, args },
            Inline(value),
        }
        ProcedureReference => visit_procedure_reference, walk_procedure_reference {
            Name(value),
            Parameter(value),
        }
        YieldItem => visit_yield_item, walk_yield_item {
            All,
            Item { name, alias },
        }
        ResultKind => visit_result_kind, walk_result_kind {
            Return,
            Select,
            Finish,
        }
        SortDirection => visit_sort_direction, walk_sort_direction {
            Asc,
            Desc,
        }
        NullOrdering => visit_null_ordering, walk_null_ordering {
            First,
            Last,
        }
        SetItem => visit_set_item, walk_set_item {
            Property { target, value },
            AllProperties { variable, properties },
            Label { variable, label },
        }
        RemoveItem => visit_remove_item, walk_remove_item {
            Property(value),
            Label { variable, label },
        }
        SessionSetTarget => visit_session_set_target, walk_session_set_target {
            Schema(value),
            Graph(value),
            TimeZone(value),
            GraphParameter(value),
            BindingTableParameter(value),
            ValueParameter(value),
        }
        SessionResetTarget => visit_session_reset_target, walk_session_reset_target {
            AllCharacteristics,
            AllParameters,
            Schema,
            Graph,
            TimeZone,
            Parameter(value),
        }
        CreateGraphType => visit_create_graph_type, walk_create_graph_type {
            Any { typed, property_graph },
            Like(value),
            Named { typed, name },
            Nested { typed, property_graph, body },
        }
        CreateGraphTypeSource => visit_create_graph_type_source, walk_create_graph_type_source {
            CopyOf(value),
            CopyOfExternal(value),
            Like(value),
            Nested(value),
        }
        TransactionAccessMode => visit_transaction_access_mode, walk_transaction_access_mode {
            ReadOnly,
            ReadWrite,
        }
        IsolationLevel => visit_isolation_level, walk_isolation_level {
            Serializable,
            RepeatableRead,
            ReadCommitted,
            ReadUncommitted,
        }
        GraphExpression => visit_graph_expression, walk_graph_expression {
            Variable(value),
            Name(value),
            Parameter(value),
            CurrentGraph,
            CurrentPropertyGraph,
            HomeGraph,
            HomePropertyGraph,
        }
        BindingTableExpression => visit_binding_table_expression, walk_binding_table_expression {
            NestedQuery(value),
            Variable(value),
            Name(value),
            Parameter(value),
        }
        GraphTypeReference => visit_graph_type_reference, walk_graph_type_reference {
            Name(value),
            Parameter(value),
        }
        SchemaReference => visit_schema_reference, walk_schema_reference {
            Name(value),
            Absolute(value),
            Root,
            Parent { levels, name },
            Parameter(value),
            CurrentSchema,
            HomeSchema,
        }
        PathPatternFactor => visit_path_pattern_factor, walk_path_pattern_factor {
            Node(value),
            Relationship(value),
            Parenthesized(value),
            Alternation { alternation, alternatives },
        }
        PathPatternAlternation => visit_path_pattern_alternation, walk_path_pattern_alternation {
            Union,
            Multiset,
        }
        PathPatternPrefix => visit_path_pattern_prefix, walk_path_pattern_prefix {
            Mode { mode, path_or_paths },
            Search(value),
        }
        PathMode => visit_path_mode, walk_path_mode {
            Walk,
            Trail,
            Simple,
            Acyclic,
        }
        PathOrPaths => visit_path_or_paths, walk_path_or_paths {
            Path,
            Paths,
        }
        PathSearchPrefix => visit_path_search_prefix, walk_path_search_prefix {
            All { mode, path_or_paths },
            Any { count, mode, path_or_paths },
            Shortest { mode, path_or_paths },
            AllShortest { mode, path_or_paths },
            AnyShortest { mode, path_or_paths },
            CountedShortest { count, mode, path_or_paths },
            CountedShortestGroup { count, mode, path_or_paths, groups },
        }
        UnsignedIntegerSpecification => visit_unsigned_integer_specification, walk_unsigned_integer_specification {
            Literal(value),
            Parameter(value),
        }
        Direction => visit_direction, walk_direction {
            Left,
            Right,
            Undirected,
            LeftOrUndirected,
            UndirectedOrRight,
            LeftOrRight,
            Any,
        }
        LabelExpression => visit_label_expression, walk_label_expression {
            Label(value),
            Wildcard,
            And(left, right),
            Or(left, right),
            Not(value),
            Parenthesized(value),
        }
        PathPatternQuantifier => visit_path_pattern_quantifier, walk_path_pattern_quantifier {
            ZeroOrMore,
            OneOrMore,
            Optional,
            Fixed(value),
            Range { min, max },
        }
        GraphTypeElement => visit_graph_type_element, walk_graph_type_element {
            Node(value),
            Edge(value),
        }
        ValueType => visit_value_type, walk_value_type {
            Named { name, parameters },
            CharacterString { kind, max_length },
            Boolean { kind },
            Temporal { kind },
            ByteString { kind, min_length, max_length },
            ExactNumeric { kind, signed, precision, scale },
            ApproximateNumeric { kind, precision, scale },
            List(value),
            ListWithLength { element, max_length },
            Array(value),
            ArrayWithLength { element, max_length },
            Path,
            AnyRecord,
            Record(value),
            AnyDynamic,
            PropertyValue,
            DynamicUnion(value),
            GraphReference { property_graph, body },
            NodeReference { definition },
            EdgeReference { definition },
            BindingTable { binding, fields },
            NotNull(value),
        }
        CharacterStringTypeKind => visit_character_string_type_kind, walk_character_string_type_kind {
            String,
            Varchar,
        }
        BooleanTypeKind => visit_boolean_type_kind, walk_boolean_type_kind {
            Bool,
            Boolean,
        }
        TemporalTypeKind => visit_temporal_type_kind, walk_temporal_type_kind {
            ZonedDateTime,
            LocalDateTime,
            Date,
            ZonedTime,
            LocalTime,
            Duration,
        }
        ByteStringTypeKind => visit_byte_string_type_kind, walk_byte_string_type_kind {
            Bytes,
            Binary,
            Varbinary,
        }
        ExactNumericTypeKind => visit_exact_numeric_type_kind, walk_exact_numeric_type_kind {
            Decimal,
            Integer,
        }
        ApproximateNumericTypeKind => visit_approximate_numeric_type_kind, walk_approximate_numeric_type_kind {
            Float,
            Real,
            Double,
        }
        Expr => visit_expr, walk_expr {
            Identifier(value),
            Property { base, key },
            GraphReference { property_graph, graph },
            BindingTableReference { binding, table },
            Parameter(value),
            Literal(value),
            Unary { op, expr },
            Binary { left, op, right },
            Function { name, quantifier, args },
            SubstringByLength { side, value, length },
            Trim { specification, trim_character, value },
            MultiTrim { specification, value, trim_characters },
            TrimList { value, count },
            Elements { path },
            StringLength { unit, value },
            Fold { upper, value },
            Normalize { value, normal_form },
            CurrentDate,
            CurrentTime { precision },
            CurrentTimestamp { precision },
            CurrentUser,
            DateTimeFunction { function, value },
            LocalTime { precision },
            LocalTimestamp { precision },
            AbsoluteValue { expr },
            DurationFunction { value },
            DurationBetween { left, right },
            NumericFunction { function, args },
            Cast { expr, value_type },
            ValueQuery { query },
            Let { items, value },
            IsTyped { expr, negated, value_type },
            IsNull { expr, negated },
            IsUnknown { expr, negated },
            IsTruth { expr, negated, value },
            PropertyExists { variable, property },
            ElementId { variable },
            Exists { patterns },
            ExistsQuery { query },
            ExistsMatch { matches },
            Same { variables },
            AllDifferent { variables },
            SourceDestination { node, negated, kind, edge },
            IsNormalized { expr, negated, normal_form },
            IsDirected { variable, negated },
            IsLabeled { variable, negated, label_expression },
            Case { operand, when_clauses, else_expr },
            Coalesce(value),
            NullIf { left, right },
            List(value),
            TypedList { type_name, values },
            Path(value),
            Record { explicit, fields },
            Map(value),
            Wildcard,
        }
        TrimSpec => visit_trim_spec, walk_trim_spec {
            Leading,
            Trailing,
            Both,
        }
        StringLengthUnit => visit_string_length_unit, walk_string_length_unit {
            Characters,
            Bytes,
        }
        SubstringSide => visit_substring_side, walk_substring_side {
            Left,
            Right,
        }
        NumericFunctionKind => visit_numeric_function_kind, walk_numeric_function_kind {
            Mod,
            Floor,
            Ceil,
            Sqrt,
            Power,
            Log,
            Log10,
            Ln,
            Exp,
            Sin,
            Cos,
            Tan,
            Cot,
            Sinh,
            Cosh,
            Tanh,
            Asin,
            Acos,
            Atan,
            Degrees,
            Radians,
            PathLength,
        }
        DateTimeFunctionKind => visit_date_time_function_kind, walk_date_time_function_kind {
            Date,
            ZonedTime,
            ZonedDateTime,
            LocalTime,
            LocalDateTime,
        }
        ListValueTypeName => visit_list_value_type_name, walk_list_value_type_name {
            List,
            Array,
        }
        SourceDestinationKind => visit_source_destination_kind, walk_source_destination_kind {
            Source,
            Destination,
        }
        Literal => visit_literal, walk_literal {
            Null,
            Boolean(value),
            Unknown,
            Integer(value),
            Decimal(value),
            String(value),
            Bytes(value),
            Date(value),
            Time(value),
            DateTime(value),
            Duration(value),
            SqlInterval { value, qualifier },
        }
        UnaryOp => visit_unary_op, walk_unary_op {
            Not,
            Pos,
            Neg,
        }
        BinaryOp => visit_binary_op, walk_binary_op {
            Or,
            Xor,
            And,
            Eq,
            Neq,
            Lt,
            Le,
            Gt,
            Ge,
            Concat,
            Add,
            Sub,
            Mul,
            Div,
        }
    }
}
