use crate::ast::*;
use crate::error::{Error, Result};
use crate::lexer::Lexer;
use crate::token::{Token, TokenKind};

type EdgeTypeTail = (
    Direction,
    Option<Identifier>,
    Option<Identifier>,
    Vec<PropertyTypeDefinition>,
);
type PatternElementBody = (
    Option<Identifier>,
    bool,
    Vec<Identifier>,
    Option<LabelExpression>,
    Option<MapLiteral>,
    Option<Expr>,
);
type InsertElementBody = (
    Option<Identifier>,
    Vec<Identifier>,
    Option<LabelExpression>,
    Option<MapLiteral>,
);

pub fn parse(input: &str) -> Result<Program> {
    Parser::new(input).parse_program()
}

#[derive(Clone, Debug)]
pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    lexer_error: Option<Error>,
    stop_in_as_delimiter: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GraphTypeElementContext {
    GraphTypeSpecificationBody,
    ClosedReferenceValueType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LiteralKind {
    Date,
    Time,
    DateTime,
    Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SqlIntervalFieldKind {
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SimplifiedPrefix {
    Minus,
    ArrowLeft,
    Tilde,
    LtTilde,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InsertElementKind {
    Node,
    Edge,
}

#[derive(Clone, Debug)]
struct SimplifiedRelationshipBody {
    labels: Vec<Identifier>,
    label_expression: LabelExpression,
    direction_override: Option<Direction>,
    quantifier: Option<PathPatternQuantifier>,
}

#[derive(Clone, Debug)]
enum SimplifiedPathPatternExpression {
    Linear(Vec<RelationshipPattern>),
    Alternation {
        alternation: PathPatternAlternation,
        alternatives: Vec<Vec<RelationshipPattern>>,
    },
}

impl Parser {
    pub fn new(input: &str) -> Self {
        let mut tokens = Vec::new();
        let mut lexer_error = None;
        for token in Lexer::new(input) {
            match token {
                Ok(token) => tokens.push(token),
                Err(err) => {
                    lexer_error = Some(err);
                    break;
                }
            }
        }
        tokens.push(Token::new(TokenKind::Eof, "", input.len()));
        Self {
            tokens,
            pos: 0,
            lexer_error,
            stop_in_as_delimiter: false,
        }
    }

    pub fn parse_program(mut self) -> Result<Program> {
        let program = self.parse_program_body(TokenKind::Eof)?;
        if let Some(err) = self.lexer_error {
            Err(err)
        } else {
            Ok(program)
        }
    }

    fn parse_program_body(&mut self, terminator: TokenKind) -> Result<Program> {
        let at_schema = self.parse_optional_program_at_schema_clause()?;
        let mut definitions = Vec::new();
        let mut statements = Vec::new();
        self.eat_many(TokenKind::Semicolon);
        while self.starts_binding_variable_definition() {
            let offset = self.peek().offset;
            let definition = self.parse_binding_variable_definition()?;
            self.push_binding_variable_definition(&mut definitions, definition, offset)?;
            self.eat_many(TokenKind::Semicolon);
        }
        while !self.at(terminator) {
            let statement = self.parse_statement()?;
            let can_continue_without_semicolon =
                self.can_continue_program_without_semicolon(&statement, terminator);
            statements.push(statement);
            while self.eat(TokenKind::Next).is_some() {
                statements.push(Statement::Next(self.parse_next_statement()?));
            }
            if !matches!(self.peek_kind(), TokenKind::Semicolon) && !self.at(terminator) {
                if can_continue_without_semicolon {
                    continue;
                }
                return Err(
                    self.error_expected(vec![TokenKind::Semicolon, terminator], self.peek_kind())
                );
            }
            self.eat_many(TokenKind::Semicolon);
        }
        Ok(Program {
            at_schema,
            definitions,
            statements,
        })
    }

    fn can_continue_program_without_semicolon(
        &self,
        previous: &Statement,
        terminator: TokenKind,
    ) -> bool {
        if terminator != TokenKind::Eof {
            return false;
        }

        match previous {
            Statement::StartTransaction(_) => {
                starts_transaction_procedure(self.peek_kind())
                    || self.starts_session_close_command()
            }
            Statement::Query(_)
            | Statement::Call(_)
            | Statement::Insert(_)
            | Statement::Delete(_)
            | Statement::Set(_)
            | Statement::Remove(_)
            | Statement::LinearCatalog(_)
            | Statement::CreateGraph(_)
            | Statement::DropGraph(_)
            | Statement::CreateGraphType(_)
            | Statement::DropGraphType(_)
            | Statement::CreateSchema(_)
            | Statement::DropSchema(_) => {
                matches!(self.peek_kind(), TokenKind::Commit | TokenKind::Rollback)
                    || self.starts_session_close_command()
            }
            Statement::Commit(_) | Statement::Rollback(_) => self.starts_session_close_command(),
            Statement::SessionSet(_) | Statement::SessionReset(_) => {
                self.starts_session_activity_command() || self.starts_session_close_command()
            }
            _ => false,
        }
    }

    fn starts_session_activity_command(&self) -> bool {
        self.peek_kind() == TokenKind::Session
            && matches!(self.peek_n_kind(1), Some(TokenKind::Set | TokenKind::Reset))
    }

    fn starts_session_close_command(&self) -> bool {
        self.peek_kind() == TokenKind::Session && self.peek_n_kind(1) == Some(TokenKind::Close)
    }

    fn parse_optional_program_at_schema_clause(&mut self) -> Result<Option<SchemaReference>> {
        if !self.at(TokenKind::At) {
            return Ok(None);
        }

        let mut trial = self.clone();
        let at_schema = trial.parse_optional_at_schema_clause()?;
        trial.eat_many(TokenKind::Semicolon);
        if trial.starts_binding_variable_definition() {
            *self = trial;
            Ok(at_schema)
        } else {
            Ok(None)
        }
    }

    fn starts_binding_variable_definition(&self) -> bool {
        matches!(
            self.peek_kind(),
            TokenKind::Value | TokenKind::Graph | TokenKind::Table
        ) || (self.peek_n_kind(0) == Some(TokenKind::Binding)
            && self.peek_n_kind(1) == Some(TokenKind::Table))
            || (self.peek_n_kind(0) == Some(TokenKind::Property)
                && self.peek_n_kind(1) == Some(TokenKind::Graph))
    }

    fn parse_binding_variable_definition(&mut self) -> Result<BindingVariableDefinition> {
        if self.at(TokenKind::Value) {
            return Ok(BindingVariableDefinition::Value(
                self.parse_value_variable_definition()?,
            ));
        }
        if self.at(TokenKind::Binding) || self.at(TokenKind::Table) {
            return Ok(BindingVariableDefinition::BindingTable(
                self.parse_binding_table_variable_definition()?,
            ));
        }
        Ok(BindingVariableDefinition::Graph(
            self.parse_graph_variable_definition()?,
        ))
    }

    fn push_binding_variable_definition(
        &self,
        definitions: &mut Vec<BindingVariableDefinition>,
        definition: BindingVariableDefinition,
        offset: usize,
    ) -> Result<()> {
        let name = binding_variable_definition_name(&definition);
        if definitions.iter().any(|existing| {
            binding_variable_definition_name(existing)
                .value
                .eq_ignore_ascii_case(&name.value)
        }) {
            return Err(Error::Message {
                offset,
                message: format!("duplicate binding variable '{}'", name.value),
            });
        }
        definitions.push(definition);
        Ok(())
    }

    fn parse_graph_variable_definition(&mut self) -> Result<GraphVariableDefinition> {
        let property_graph = if self.eat(TokenKind::Property).is_some() {
            self.expect(TokenKind::Graph)?;
            true
        } else {
            self.expect(TokenKind::Graph)?;
            false
        };
        let name = self.parse_identifier()?;
        let (typed, value_type) = self.parse_optional_definition_value_type()?;
        self.expect(TokenKind::Eq)?;
        Ok(GraphVariableDefinition {
            property_graph,
            name,
            typed,
            value_type,
            initializer: self.parse_graph_expression()?,
        })
    }

    fn parse_binding_table_variable_definition(
        &mut self,
    ) -> Result<BindingTableVariableDefinition> {
        let binding = self.eat(TokenKind::Binding).is_some();
        self.expect(TokenKind::Table)?;
        let name = self.parse_identifier()?;
        let (typed, value_type) = self.parse_optional_definition_value_type()?;
        self.expect(TokenKind::Eq)?;
        Ok(BindingTableVariableDefinition {
            binding,
            name,
            typed,
            value_type,
            initializer: self.parse_binding_table_expression()?,
        })
    }

    fn parse_value_variable_definition(&mut self) -> Result<ValueVariableDefinition> {
        self.expect(TokenKind::Value)?;
        let name = self.parse_identifier()?;
        let (typed, value_type) = self.parse_optional_definition_value_type()?;
        self.expect(TokenKind::Eq)?;
        Ok(ValueVariableDefinition {
            name,
            typed,
            value_type,
            initializer: self.parse_expr()?,
        })
    }

    fn parse_optional_definition_value_type(&mut self) -> Result<(bool, Option<ValueType>)> {
        if self.at(TokenKind::Eq) {
            return Ok((false, None));
        }
        let typed = self.parse_optional_typed_marker()?;
        Ok((typed, Some(self.parse_value_type()?)))
    }

    fn parse_next_statement(&mut self) -> Result<NextStatement> {
        let yield_clause = if self.eat(TokenKind::Yield).is_some() {
            Some(self.parse_yield_clause_body(true)?)
        } else {
            None
        };
        Ok(NextStatement {
            yield_clause,
            statement: Box::new(self.parse_statement()?),
        })
    }

    fn parse_statement(&mut self) -> Result<Statement> {
        match self.peek_kind() {
            TokenKind::At => Ok(Statement::Query(self.parse_query_statement()?)),
            TokenKind::Use
            | TokenKind::Match
            | TokenKind::Filter
            | TokenKind::Let
            | TokenKind::For
            | TokenKind::Order
            | TokenKind::Offset
            | TokenKind::Skip
            | TokenKind::Limit
            | TokenKind::LBrace
            | TokenKind::Return
            | TokenKind::Select
            | TokenKind::Finish => Ok(Statement::Query(self.parse_query_statement()?)),
            TokenKind::Optional if self.peek_n_kind(1) == Some(TokenKind::Call) => {
                self.parse_call_statement_or_query()
            }
            TokenKind::Optional => Ok(Statement::Query(self.parse_query_statement()?)),
            TokenKind::Call => self.parse_call_statement_or_catalog_or_query(),
            TokenKind::Start => Ok(Statement::StartTransaction(
                self.parse_start_transaction_statement()?,
            )),
            TokenKind::Commit => Ok(Statement::Commit(self.parse_commit_statement()?)),
            TokenKind::Rollback => Ok(Statement::Rollback(self.parse_rollback_statement()?)),
            TokenKind::Insert => self.parse_data_modifying_statement_or_query(),
            TokenKind::Delete | TokenKind::Detach | TokenKind::Nodetach => {
                self.parse_data_modifying_statement_or_query()
            }
            TokenKind::Set => self.parse_data_modifying_statement_or_query(),
            TokenKind::Session => self.parse_session_prefixed_command(),
            TokenKind::Remove => self.parse_data_modifying_statement_or_query(),
            TokenKind::Create | TokenKind::Drop => {
                self.parse_catalog_modifying_statement_or_linear()
            }
            kind => Err(self.error_expected(
                vec![
                    TokenKind::Use,
                    TokenKind::At,
                    TokenKind::Match,
                    TokenKind::Optional,
                    TokenKind::Filter,
                    TokenKind::Let,
                    TokenKind::For,
                    TokenKind::Order,
                    TokenKind::Offset,
                    TokenKind::Skip,
                    TokenKind::Limit,
                    TokenKind::Call,
                    TokenKind::Return,
                    TokenKind::Select,
                    TokenKind::Finish,
                    TokenKind::Start,
                    TokenKind::Commit,
                    TokenKind::Rollback,
                    TokenKind::Insert,
                    TokenKind::Delete,
                    TokenKind::Set,
                    TokenKind::Session,
                    TokenKind::Reset,
                    TokenKind::Close,
                    TokenKind::Remove,
                    TokenKind::Create,
                    TokenKind::Drop,
                ],
                kind,
            )),
        }
    }

    fn parse_catalog_modifying_statement_or_linear(&mut self) -> Result<Statement> {
        let mut statements = vec![self.parse_simple_catalog_modifying_statement()?];
        while self.starts_simple_catalog_modifying_statement() {
            statements.push(self.parse_simple_catalog_modifying_statement()?);
        }

        if statements.len() == 1 {
            Ok(statements.remove(0))
        } else {
            Ok(Statement::LinearCatalog(statements))
        }
    }

    fn parse_simple_catalog_modifying_statement(&mut self) -> Result<Statement> {
        match self.peek_kind() {
            TokenKind::Create => self.parse_create_statement(),
            TokenKind::Drop => self.parse_drop_statement(),
            TokenKind::Call => Ok(Statement::Call(CallStatement {
                call: self.parse_call_clause()?,
            })),
            kind => Err(self.error_expected(
                vec![TokenKind::Create, TokenKind::Drop, TokenKind::Call],
                kind,
            )),
        }
    }

    fn starts_simple_catalog_modifying_statement(&self) -> bool {
        match self.peek_kind() {
            TokenKind::Create => matches!(
                self.peek_n_kind(1),
                Some(TokenKind::Or | TokenKind::Property | TokenKind::Graph | TokenKind::Schema)
            ),
            TokenKind::Drop => matches!(
                self.peek_n_kind(1),
                Some(TokenKind::Property | TokenKind::Graph | TokenKind::Schema)
            ),
            TokenKind::Call => true,
            _ => false,
        }
    }

    fn parse_query_statement(&mut self) -> Result<QueryStatement> {
        let at_schema = self.parse_optional_at_schema_clause()?;
        let use_graph = if self.at(TokenKind::Use) {
            Some(self.parse_use_graph_clause()?)
        } else {
            None
        };
        let body = self.parse_query_body()?;
        let mut set_operations = Vec::new();
        let mut query_conjunction = None;
        while matches!(
            self.peek_kind(),
            TokenKind::Union | TokenKind::Except | TokenKind::Intersect | TokenKind::Otherwise
        ) {
            let operator_offset = self.peek().offset;
            let (operator, quantifier) = self.parse_query_conjunction()?;
            if let Some(expected) = query_conjunction {
                if operator != expected {
                    return Err(Error::Message {
                        offset: operator_offset,
                        message: "mixed query conjunctions are not allowed".to_owned(),
                    });
                }
            } else {
                query_conjunction = Some(operator);
            }
            let body = self.parse_query_body()?;
            set_operations.push(QuerySetOperation {
                operator,
                quantifier,
                body,
            });
        }
        Ok(QueryStatement {
            at_schema,
            use_graph,
            body,
            set_operations,
        })
    }

    fn parse_query_conjunction(&mut self) -> Result<(QuerySetOperator, Option<SetQuantifier>)> {
        if self.eat(TokenKind::Otherwise).is_some() {
            return Ok((QuerySetOperator::Otherwise, None));
        }

        let operator = if self.eat(TokenKind::Union).is_some() {
            QuerySetOperator::Union
        } else if self.eat(TokenKind::Except).is_some() {
            QuerySetOperator::Except
        } else if self.eat(TokenKind::Intersect).is_some() {
            QuerySetOperator::Intersect
        } else {
            return Err(self.error_expected(
                vec![
                    TokenKind::Union,
                    TokenKind::Except,
                    TokenKind::Intersect,
                    TokenKind::Otherwise,
                ],
                self.peek_kind(),
            ));
        };

        let quantifier = if self.eat(TokenKind::All).is_some() {
            Some(SetQuantifier::All)
        } else {
            self.eat(TokenKind::Distinct);
            Some(SetQuantifier::Distinct)
        };

        Ok((operator, quantifier))
    }

    fn parse_optional_at_schema_clause(&mut self) -> Result<Option<SchemaReference>> {
        if self.eat(TokenKind::At).is_none() {
            return Ok(None);
        }
        self.expect(TokenKind::Schema)?;
        Ok(Some(self.parse_schema_reference()?))
    }

    fn parse_call_statement_or_query(&mut self) -> Result<Statement> {
        let mut trial = self.clone();
        let call = trial.parse_call_clause()?;
        if is_statement_follow(trial.peek_kind()) {
            *self = trial;
            Ok(Statement::Call(CallStatement { call }))
        } else {
            Ok(Statement::Query(self.parse_query_statement()?))
        }
    }

    fn parse_call_statement_or_catalog_or_query(&mut self) -> Result<Statement> {
        let mut trial = self.clone();
        let call = trial.parse_call_clause()?;
        if is_statement_follow(trial.peek_kind()) {
            *self = trial;
            return Ok(Statement::Call(CallStatement { call }));
        }

        if trial.starts_simple_catalog_modifying_statement() {
            *self = trial;
            let mut statements = vec![Statement::Call(CallStatement { call })];
            while self.starts_simple_catalog_modifying_statement() {
                statements.push(self.parse_simple_catalog_modifying_statement()?);
            }
            return Ok(Statement::LinearCatalog(statements));
        }

        Ok(Statement::Query(self.parse_query_statement()?))
    }

    fn parse_data_modifying_statement_or_query(&mut self) -> Result<Statement> {
        let mut trial = self.clone();
        let statement = match trial.peek_kind() {
            TokenKind::Insert => Statement::Insert(trial.parse_insert_statement()?),
            TokenKind::Delete | TokenKind::Detach | TokenKind::Nodetach => {
                Statement::Delete(trial.parse_delete_statement()?)
            }
            TokenKind::Set => Statement::Set(trial.parse_set_statement()?),
            TokenKind::Remove => Statement::Remove(trial.parse_remove_statement()?),
            kind => {
                return Err(trial.error_expected(
                    vec![
                        TokenKind::Insert,
                        TokenKind::Delete,
                        TokenKind::Detach,
                        TokenKind::Nodetach,
                        TokenKind::Set,
                        TokenKind::Remove,
                    ],
                    kind,
                ));
            }
        };
        if is_statement_follow(trial.peek_kind()) {
            *self = trial;
            Ok(statement)
        } else {
            Ok(Statement::Query(self.parse_query_statement()?))
        }
    }

    fn parse_query_body(&mut self) -> Result<QueryBody> {
        let mut clauses = Vec::new();
        while matches!(
            self.peek_kind(),
            TokenKind::Use
                | TokenKind::Optional
                | TokenKind::Match
                | TokenKind::Filter
                | TokenKind::Let
                | TokenKind::For
                | TokenKind::Call
                | TokenKind::Insert
                | TokenKind::Delete
                | TokenKind::Detach
                | TokenKind::Nodetach
                | TokenKind::Set
                | TokenKind::Remove
                | TokenKind::Order
                | TokenKind::Offset
                | TokenKind::Skip
                | TokenKind::Limit
                | TokenKind::LBrace
        ) {
            match self.peek_kind() {
                TokenKind::Use => {
                    clauses.push(QueryClause::UseGraph(self.parse_use_graph_clause()?));
                }
                TokenKind::LBrace => {
                    clauses.push(QueryClause::NestedQuery(Box::new(
                        self.parse_nested_query_specification()?,
                    )));
                }
                TokenKind::Optional | TokenKind::Match => {
                    if self.at(TokenKind::Optional) && self.peek_n_kind(1) == Some(TokenKind::Call)
                    {
                        clauses.push(QueryClause::Call(self.parse_call_clause()?));
                    } else {
                        clauses.push(self.parse_match_query_clause()?);
                    }
                }
                TokenKind::Filter => {
                    clauses.push(QueryClause::Filter(self.parse_filter_clause()?));
                }
                TokenKind::Let => {
                    clauses.push(QueryClause::Let(self.parse_let_clause()?));
                }
                TokenKind::For => {
                    clauses.push(QueryClause::For(self.parse_for_clause()?));
                }
                TokenKind::Call => {
                    clauses.push(QueryClause::Call(self.parse_call_clause()?));
                }
                TokenKind::Insert => {
                    clauses.push(QueryClause::Insert(self.parse_insert_statement()?));
                }
                TokenKind::Delete | TokenKind::Detach | TokenKind::Nodetach => {
                    clauses.push(QueryClause::Delete(self.parse_delete_statement()?));
                }
                TokenKind::Set => {
                    clauses.push(QueryClause::Set(self.parse_set_statement()?));
                }
                TokenKind::Remove => {
                    clauses.push(QueryClause::Remove(self.parse_remove_statement()?));
                }
                TokenKind::Order | TokenKind::Offset | TokenKind::Skip | TokenKind::Limit => {
                    clauses.push(QueryClause::OrderByPage(self.parse_order_by_page_clause()?));
                }
                _ => unreachable!(),
            }
        }
        if clauses.is_empty()
            && !matches!(
                self.peek_kind(),
                TokenKind::Return | TokenKind::Select | TokenKind::Finish
            )
        {
            return Err(self.error_expected(
                vec![
                    TokenKind::Match,
                    TokenKind::Optional,
                    TokenKind::Filter,
                    TokenKind::Let,
                    TokenKind::For,
                    TokenKind::Call,
                    TokenKind::Insert,
                    TokenKind::Delete,
                    TokenKind::Detach,
                    TokenKind::Nodetach,
                    TokenKind::Set,
                    TokenKind::Remove,
                    TokenKind::Order,
                    TokenKind::Offset,
                    TokenKind::Skip,
                    TokenKind::Limit,
                    TokenKind::Use,
                    TokenKind::LBrace,
                    TokenKind::Return,
                    TokenKind::Select,
                    TokenKind::Finish,
                ],
                self.peek_kind(),
            ));
        }

        let result_clause = if !matches!(
            self.peek_kind(),
            TokenKind::Return | TokenKind::Select | TokenKind::Finish
        ) && (clauses.iter().any(is_data_modifying_query_clause)
            || clauses.iter().any(is_nested_query_clause))
            && matches!(
                self.peek_kind(),
                TokenKind::Semicolon
                    | TokenKind::Eof
                    | TokenKind::Next
                    | TokenKind::RBrace
                    | TokenKind::Union
                    | TokenKind::Except
                    | TokenKind::Intersect
                    | TokenKind::Otherwise
            ) {
            // A nested read query is a query primary: its result flows outward.
            // Only a modifying pipeline implicitly finishes without a result.
            let inherit = clauses.last().is_some_and(is_nested_query_clause)
                && !clauses.iter().any(is_data_modifying_query_clause);
            ResultClause {
                kind: if inherit {
                    ResultKind::Return
                } else {
                    ResultKind::Finish
                },
                quantifier: None,
                distinct: false,
                items: if inherit {
                    vec![ResultItem {
                        expr: Expr::Wildcard,
                        alias: None,
                    }]
                } else {
                    Vec::new()
                },
            }
        } else {
            self.parse_result_clause()?
        };
        if result_clause.kind == ResultKind::Finish {
            return Ok(QueryBody {
                clauses,
                result_clause,
                select_from: Vec::new(),
                select_query: None,
                select_where: None,
                group_by: Vec::new(),
                empty_grouping_set: false,
                having: None,
                order_by: Vec::new(),
                offset: None,
                limit: None,
            });
        }
        if result_clause.kind == ResultKind::Return
            && result_clause_has_wildcard(&result_clause)
            && !query_clauses_have_non_unit_working_table(&clauses)
        {
            return Err(Error::Message {
                offset: self.peek().offset,
                message: "RETURN * requires a non-unit incoming working table".to_owned(),
            });
        }
        let (select_from, select_query) =
            if result_clause.kind == ResultKind::Select && self.eat(TokenKind::From).is_some() {
                self.parse_select_from_clause()?
            } else {
                (Vec::new(), None)
            };
        if result_clause.kind == ResultKind::Select
            && result_clause_has_wildcard(&result_clause)
            && select_from.is_empty()
            && select_query.is_none()
        {
            return Err(Error::Message {
                offset: self.peek().offset,
                message: "SELECT * requires a FROM clause".to_owned(),
            });
        }
        let select_where =
            if result_clause.kind == ResultKind::Select && self.eat(TokenKind::Where).is_some() {
                Some(self.parse_expr()?)
            } else {
                None
            };
        let mut group_by = Vec::new();
        let mut empty_grouping_set = false;
        if self.eat(TokenKind::Group).is_some() {
            self.expect(TokenKind::By)?;
            if self.eat(TokenKind::LParen).is_some() {
                self.expect(TokenKind::RParen)?;
                empty_grouping_set = true;
            } else {
                loop {
                    group_by.push(self.parse_grouping_element()?);
                    if self.eat(TokenKind::Comma).is_none() {
                        break;
                    }
                }
            }
        }
        if result_clause_has_wildcard(&result_clause)
            && (!group_by.is_empty() || empty_grouping_set)
        {
            return Err(Error::Message {
                offset: self.peek().offset,
                message: "asterisk result cannot contain GROUP BY".to_owned(),
            });
        }
        let having = if self.at(TokenKind::Having) {
            if result_clause.kind != ResultKind::Select {
                return Err(Error::Message {
                    offset: self.peek().offset,
                    message: "HAVING is only valid in SELECT statements".to_owned(),
                });
            }
            self.expect(TokenKind::Having)?;
            Some(self.parse_expr()?)
        } else {
            None
        };
        let page = if matches!(
            self.peek_kind(),
            TokenKind::Order | TokenKind::Offset | TokenKind::Skip | TokenKind::Limit
        ) {
            self.parse_order_by_page_clause()?
        } else {
            OrderByPageClause {
                order_by: Vec::new(),
                offset: None,
                limit: None,
            }
        };
        Ok(QueryBody {
            clauses,
            result_clause,
            select_from,
            select_query,
            select_where,
            group_by,
            empty_grouping_set,
            having,
            order_by: page.order_by,
            offset: page.offset,
            limit: page.limit,
        })
    }

    fn parse_order_by_page_clause(&mut self) -> Result<OrderByPageClause> {
        let order_by = if self.eat(TokenKind::Order).is_some() {
            self.expect(TokenKind::By)?;
            self.parse_order_by_list()?
        } else {
            Vec::new()
        };
        let offset = if self.eat(TokenKind::Offset).is_some() || self.eat(TokenKind::Skip).is_some()
        {
            Some(self.parse_unsigned_integer_specification()?)
        } else {
            None
        };
        let limit = if self.eat(TokenKind::Limit).is_some() {
            Some(self.parse_unsigned_integer_specification()?)
        } else {
            None
        };
        if order_by.is_empty() && offset.is_none() && limit.is_none() {
            return Err(self.error_expected(
                vec![
                    TokenKind::Order,
                    TokenKind::Offset,
                    TokenKind::Skip,
                    TokenKind::Limit,
                ],
                self.peek_kind(),
            ));
        }
        Ok(OrderByPageClause {
            order_by,
            offset,
            limit,
        })
    }

    fn parse_order_by_list(&mut self) -> Result<Vec<OrderByItem>> {
        let mut order_by = Vec::new();
        loop {
            let expr = self.parse_expr()?;
            let direction =
                if self.eat(TokenKind::Asc).is_some() || self.eat(TokenKind::Ascending).is_some() {
                    Some(SortDirection::Asc)
                } else if self.eat(TokenKind::Desc).is_some()
                    || self.eat(TokenKind::Descending).is_some()
                {
                    Some(SortDirection::Desc)
                } else {
                    None
                };
            let null_ordering = if self.eat(TokenKind::Nulls).is_some() {
                if self.eat(TokenKind::First).is_some() {
                    Some(NullOrdering::First)
                } else if self.eat(TokenKind::Last).is_some() {
                    Some(NullOrdering::Last)
                } else {
                    return Err(self.error_expected(
                        vec![TokenKind::First, TokenKind::Last],
                        self.peek_kind(),
                    ));
                }
            } else {
                None
            };
            order_by.push(OrderByItem {
                expr,
                direction,
                null_ordering,
            });
            if self.eat(TokenKind::Comma).is_none() {
                break;
            }
        }
        Ok(order_by)
    }

    fn parse_grouping_element(&mut self) -> Result<Expr> {
        let offset = self.peek().offset;
        let identifier = self.parse_identifier()?;
        if matches!(
            self.peek_kind(),
            TokenKind::Dot
                | TokenKind::Plus
                | TokenKind::Minus
                | TokenKind::Star
                | TokenKind::Slash
                | TokenKind::Percent
                | TokenKind::LParen
                | TokenKind::LBracket
        ) {
            return Err(Error::Message {
                offset,
                message: "GROUP BY elements must be binding variable references".to_owned(),
            });
        }
        Ok(Expr::Identifier(identifier))
    }

    fn parse_select_from_clause(
        &mut self,
    ) -> Result<(Vec<SelectGraphMatch>, Option<SelectQuerySpecification>)> {
        if self.at(TokenKind::LBrace) {
            return Ok((
                Vec::new(),
                Some(SelectQuerySpecification {
                    graph: None,
                    query: Box::new(self.parse_nested_query_specification()?),
                }),
            ));
        }

        let graph = self.parse_graph_expression()?;
        if self.at(TokenKind::LBrace) {
            return Ok((
                Vec::new(),
                Some(SelectQuerySpecification {
                    graph: Some(graph),
                    query: Box::new(self.parse_nested_query_specification()?),
                }),
            ));
        }

        let match_clause = self.parse_match_clause()?;
        let mut items = vec![SelectGraphMatch {
            graph,
            match_clause,
        }];
        while self.eat(TokenKind::Comma).is_some() {
            let graph = self.parse_graph_expression()?;
            let match_clause = self.parse_match_clause()?;
            items.push(SelectGraphMatch {
                graph,
                match_clause,
            });
        }
        Ok((items, None))
    }

    fn parse_nested_query_specification(&mut self) -> Result<QueryStatement> {
        self.expect(TokenKind::LBrace)?;
        let query = self.parse_query_statement()?;
        self.expect(TokenKind::RBrace)?;
        Ok(query)
    }

    fn parse_use_graph_clause(&mut self) -> Result<GraphExpression> {
        self.expect(TokenKind::Use)?;
        if self.eat(TokenKind::Property).is_some() {
            self.expect(TokenKind::Graph)?;
        } else {
            self.eat(TokenKind::Graph);
        }
        self.parse_graph_expression()
    }

    fn parse_filter_clause(&mut self) -> Result<FilterClause> {
        self.expect(TokenKind::Filter)?;
        self.eat(TokenKind::Where);
        Ok(FilterClause {
            predicate: self.parse_expr()?,
        })
    }

    fn parse_let_clause(&mut self) -> Result<LetClause> {
        self.expect(TokenKind::Let)?;
        let mut items = Vec::new();
        loop {
            let item_offset = self.peek().offset;
            let item = self.parse_let_item()?;
            self.push_let_item(&mut items, item, item_offset)?;
            if self.eat(TokenKind::Comma).is_none() {
                break;
            }
        }
        Ok(LetClause { items })
    }

    fn parse_let_item(&mut self) -> Result<LetItem> {
        self.parse_let_item_with_in_delimiter(false)
    }

    fn parse_let_item_with_in_delimiter(&mut self, stop_at_in: bool) -> Result<LetItem> {
        if self.eat(TokenKind::Value).is_some() {
            let name = self.parse_identifier()?;
            let (typed, value_type) = self.parse_optional_definition_value_type()?;
            self.expect(TokenKind::Eq)?;
            return Ok(LetItem {
                name,
                typed,
                value_type,
                value: self.parse_expr_for_let_item(stop_at_in)?,
            });
        }
        let name = self.parse_identifier()?;
        self.expect(TokenKind::Eq)?;
        Ok(LetItem {
            name,
            typed: false,
            value_type: None,
            value: self.parse_expr_for_let_item(stop_at_in)?,
        })
    }

    fn parse_expr_for_let_item(&mut self, stop_at_in: bool) -> Result<Expr> {
        if !stop_at_in {
            return self.parse_expr();
        }
        let previous = self.stop_in_as_delimiter;
        self.stop_in_as_delimiter = true;
        let result = self.parse_expr();
        self.stop_in_as_delimiter = previous;
        result
    }

    fn parse_for_clause(&mut self) -> Result<ForClause> {
        self.expect(TokenKind::For)?;
        let variable = self.parse_identifier()?;
        self.expect(TokenKind::In)?;
        let source = self.parse_expr()?;
        let ordinality_or_offset = if self.eat(TokenKind::With).is_some() {
            let kind = if self.eat(TokenKind::Ordinality).is_some() {
                ForOrdinalityOrOffsetKind::Ordinality
            } else if self.eat(TokenKind::Offset).is_some() {
                ForOrdinalityOrOffsetKind::Offset
            } else {
                return Err(self.error_expected(
                    vec![TokenKind::Ordinality, TokenKind::Offset],
                    self.peek_kind(),
                ));
            };
            let variable_offset = self.peek().offset;
            let ordinality_variable = self.parse_identifier()?;
            if ordinality_variable
                .value
                .eq_ignore_ascii_case(&variable.value)
            {
                return Err(Error::Message {
                    offset: variable_offset,
                    message: format!(
                        "for ordinality or offset variable '{}' duplicates item alias",
                        ordinality_variable.value
                    ),
                });
            }
            Some(ForOrdinalityOrOffset {
                kind,
                variable: ordinality_variable,
            })
        } else {
            None
        };
        Ok(ForClause {
            variable,
            source,
            ordinality_or_offset,
        })
    }

    fn parse_call_clause(&mut self) -> Result<CallClause> {
        let optional = self.eat(TokenKind::Optional).is_some();
        self.expect(TokenKind::Call)?;
        let call = if matches!(self.peek_kind(), TokenKind::LBrace | TokenKind::LParen) {
            ProcedureCall::Inline(self.parse_inline_procedure_call()?)
        } else {
            let procedure = self.parse_procedure_reference()?;
            self.expect(TokenKind::LParen)?;
            let args = self.parse_procedure_argument_list()?;
            let yield_clause = if self.eat(TokenKind::Yield).is_some() {
                Some(self.parse_yield_clause_body(true)?)
            } else {
                None
            };
            return Ok(CallClause {
                optional,
                call: ProcedureCall::Named { procedure, args },
                yield_clause,
            });
        };
        Ok(CallClause {
            optional,
            call,
            yield_clause: None,
        })
    }

    fn parse_procedure_argument_list(&mut self) -> Result<Vec<Expr>> {
        let mut args = Vec::new();
        if !self.at(TokenKind::RParen) {
            loop {
                args.push(self.parse_expr()?);
                if self.eat(TokenKind::Comma).is_none() {
                    break;
                }
            }
        }
        self.expect(TokenKind::RParen)?;
        Ok(args)
    }

    fn parse_inline_procedure_call(&mut self) -> Result<InlineProcedureCall> {
        let variable_scope = if self.at(TokenKind::LParen) {
            Some(self.parse_variable_scope_clause()?)
        } else {
            None
        };
        self.expect(TokenKind::LBrace)?;
        let body = self.parse_program_body(TokenKind::RBrace)?;
        if body.statements.is_empty() {
            return Err(self.error_expected(
                vec![
                    TokenKind::Use,
                    TokenKind::At,
                    TokenKind::Match,
                    TokenKind::Optional,
                    TokenKind::Filter,
                    TokenKind::Let,
                    TokenKind::For,
                    TokenKind::Call,
                    TokenKind::Return,
                    TokenKind::Select,
                    TokenKind::Finish,
                    TokenKind::Start,
                    TokenKind::Commit,
                    TokenKind::Rollback,
                    TokenKind::Insert,
                    TokenKind::Delete,
                    TokenKind::Set,
                    TokenKind::Session,
                    TokenKind::Reset,
                    TokenKind::Close,
                    TokenKind::Remove,
                    TokenKind::Create,
                    TokenKind::Drop,
                ],
                self.peek_kind(),
            ));
        }
        self.expect(TokenKind::RBrace)?;
        Ok(InlineProcedureCall {
            variable_scope,
            body: Box::new(body),
        })
    }

    fn parse_variable_scope_clause(&mut self) -> Result<Vec<Identifier>> {
        self.expect(TokenKind::LParen)?;
        let mut variables = Vec::new();
        if !self.at(TokenKind::RParen) {
            loop {
                let offset = self.peek().offset;
                let variable = self.parse_identifier()?;
                if variables.iter().any(|existing: &Identifier| {
                    existing.value.eq_ignore_ascii_case(&variable.value)
                }) {
                    return Err(Error::Message {
                        offset,
                        message: format!(
                            "duplicate variable '{}' in variable scope",
                            variable.value
                        ),
                    });
                }
                variables.push(variable);
                if self.eat(TokenKind::Comma).is_none() {
                    break;
                }
            }
        }
        self.expect(TokenKind::RParen)?;
        Ok(variables)
    }

    fn parse_match_query_clause(&mut self) -> Result<QueryClause> {
        if self.at(TokenKind::Optional)
            && matches!(
                self.peek_n_kind(1),
                Some(TokenKind::LBrace | TokenKind::LParen)
            )
        {
            self.expect(TokenKind::Optional)?;
            let closing = if self.eat(TokenKind::LBrace).is_some() {
                TokenKind::RBrace
            } else {
                self.expect(TokenKind::LParen)?;
                TokenKind::RParen
            };
            let matches = self.parse_match_statement_block(closing)?;
            self.expect(closing)?;
            return Ok(QueryClause::OptionalMatchBlock(matches));
        }
        Ok(QueryClause::Match(self.parse_match_clause()?))
    }

    fn parse_match_clause(&mut self) -> Result<MatchClause> {
        let optional = self.eat(TokenKind::Optional).is_some();
        self.expect(TokenKind::Match)?;
        let mode = self.parse_optional_match_mode()?;
        let patterns = self.parse_pattern_list()?;
        let keep = if self.eat(TokenKind::Keep).is_some() {
            Some(self.parse_required_path_pattern_prefix()?)
        } else {
            None
        };
        let where_clause = if self.eat(TokenKind::Where).is_some() {
            Some(self.parse_expr()?)
        } else {
            None
        };
        let yield_clause = if self.eat(TokenKind::Yield).is_some() {
            Some(self.parse_yield_clause_body(false)?)
        } else {
            None
        };
        if let Some(yield_clause) = &yield_clause {
            validate_graph_pattern_yield_clause(&patterns, yield_clause, self.peek().offset)?;
        }
        Ok(MatchClause {
            optional,
            mode,
            patterns,
            keep,
            where_clause,
            yield_clause,
        })
    }

    fn parse_optional_match_mode(&mut self) -> Result<Option<MatchMode>> {
        if self.eat(TokenKind::Repeatable).is_some() {
            let bindings = if self.eat(TokenKind::Element).is_some() {
                self.eat(TokenKind::Bindings).is_some()
            } else if self.eat(TokenKind::Elements).is_some() {
                false
            } else {
                return Err(self.error_expected(
                    vec![TokenKind::Element, TokenKind::Elements],
                    self.peek_kind(),
                ));
            };
            return Ok(Some(MatchMode::RepeatableElements { bindings }));
        }

        if self.eat(TokenKind::Different).is_some() {
            let bindings = if self.eat(TokenKind::Edge).is_some()
                || self.eat(TokenKind::Relationship).is_some()
            {
                self.eat(TokenKind::Bindings).is_some()
            } else if self.eat(TokenKind::Edges).is_some()
                || self.eat(TokenKind::Relationships).is_some()
            {
                false
            } else {
                return Err(self.error_expected(
                    vec![
                        TokenKind::Edge,
                        TokenKind::Relationship,
                        TokenKind::Edges,
                        TokenKind::Relationships,
                    ],
                    self.peek_kind(),
                ));
            };
            return Ok(Some(MatchMode::DifferentEdges { bindings }));
        }

        Ok(None)
    }

    fn parse_yield_clause_body(&mut self, allow_aliases: bool) -> Result<YieldClause> {
        let mut items = Vec::new();
        let mut output_names = Vec::new();
        loop {
            let name_offset = self.peek().offset;
            let name = self.parse_identifier()?;
            let alias = if allow_aliases && self.eat(TokenKind::As).is_some() {
                let alias_offset = self.peek().offset;
                let alias = self.parse_identifier()?;
                if output_names.iter().any(|output_name: &Identifier| {
                    output_name.value.eq_ignore_ascii_case(&alias.value)
                }) {
                    return Err(Error::Message {
                        offset: alias_offset,
                        message: format!("duplicate yield output '{}'", alias.value),
                    });
                }
                output_names.push(alias.clone());
                Some(alias)
            } else {
                if output_names.iter().any(|output_name: &Identifier| {
                    output_name.value.eq_ignore_ascii_case(&name.value)
                }) {
                    return Err(Error::Message {
                        offset: name_offset,
                        message: format!("duplicate yield output '{}'", name.value),
                    });
                }
                output_names.push(name.clone());
                None
            };
            items.push(YieldItem::Item { name, alias });
            if self.eat(TokenKind::Comma).is_none() {
                break;
            }
        }
        Ok(YieldClause { items })
    }

    fn parse_result_clause(&mut self) -> Result<ResultClause> {
        let kind = if self.eat(TokenKind::Return).is_some() {
            ResultKind::Return
        } else if self.eat(TokenKind::Select).is_some() {
            ResultKind::Select
        } else if self.eat(TokenKind::Finish).is_some() {
            return Ok(ResultClause {
                kind: ResultKind::Finish,
                quantifier: None,
                distinct: false,
                items: Vec::new(),
            });
        } else {
            return Err(self.error_expected(
                vec![TokenKind::Return, TokenKind::Select, TokenKind::Finish],
                self.peek_kind(),
            ));
        };
        if kind == ResultKind::Return && self.at(TokenKind::No) {
            let offset = self.peek().offset;
            self.bump();
            return Err(Error::Message {
                offset,
                message: "RETURN NO BINDINGS is not user-visible standard GQL syntax".to_owned(),
            });
        }
        let quantifier = if self.eat(TokenKind::Distinct).is_some() {
            Some(SetQuantifier::Distinct)
        } else {
            self.eat(TokenKind::All);
            Some(SetQuantifier::All)
        };
        let distinct = quantifier == Some(SetQuantifier::Distinct);
        let mut items = Vec::new();
        let mut output_names = Vec::new();
        loop {
            let item_offset = self.peek().offset;
            let expr = self.parse_expr()?;
            let alias = if self.eat(TokenKind::As).is_some() {
                Some(self.parse_identifier()?)
            } else if let Expr::Identifier(identifier) = &expr {
                Some(identifier.clone())
            } else {
                None
            };
            if alias.is_none() && !matches!(expr, Expr::Identifier(_) | Expr::Wildcard) {
                return Err(Error::Message {
                    offset: self.peek().offset,
                    message: "non-binding result item requires AS alias".to_owned(),
                });
            }
            if let Some(output_name) = &alias {
                if output_names.iter().any(|existing: &Identifier| {
                    existing.value.eq_ignore_ascii_case(&output_name.value)
                }) {
                    return Err(Error::Message {
                        offset: item_offset,
                        message: format!("duplicate result output '{}'", output_name.value),
                    });
                }
                output_names.push(output_name.clone());
            }
            items.push(ResultItem { expr, alias });
            if self.eat(TokenKind::Comma).is_none() {
                break;
            }
        }
        Ok(ResultClause {
            kind,
            quantifier,
            distinct,
            items,
        })
    }

    fn parse_insert_statement(&mut self) -> Result<InsertStatement> {
        self.expect(TokenKind::Insert)?;
        let patterns = self.parse_insert_pattern_list()?;
        validate_insert_graph_pattern_variables(&patterns, self.peek().offset)?;
        Ok(InsertStatement { patterns })
    }

    fn parse_session_set_command(&mut self) -> Result<SessionSetCommand> {
        self.expect(TokenKind::Set)?;
        let target = if self.eat(TokenKind::Schema).is_some() {
            SessionSetTarget::Schema(self.parse_schema_reference()?)
        } else if self.eat(TokenKind::Graph).is_some() {
            if self.starts_session_set_graph_parameter() {
                SessionSetTarget::GraphParameter(self.parse_session_set_graph_parameter(false)?)
            } else {
                SessionSetTarget::Graph(self.parse_graph_expression()?)
            }
        } else if self.eat(TokenKind::Property).is_some() {
            self.expect(TokenKind::Graph)?;
            if self.starts_session_set_graph_parameter() {
                SessionSetTarget::GraphParameter(self.parse_session_set_graph_parameter(true)?)
            } else {
                SessionSetTarget::Graph(self.parse_graph_expression()?)
            }
        } else if self.at(TokenKind::Binding) || self.at(TokenKind::Table) {
            let binding = self.eat(TokenKind::Binding).is_some();
            self.expect(TokenKind::Table)?;
            SessionSetTarget::BindingTableParameter(
                self.parse_session_set_binding_table_parameter(binding)?,
            )
        } else if self.eat(TokenKind::Time).is_some() {
            self.expect(TokenKind::Zone)?;
            let value = if self.is_character_string_token(self.peek_kind()) {
                self.parse_character_string_literal_expr()?
            } else {
                self.parse_expr()?
            };
            SessionSetTarget::TimeZone(value)
        } else if self.eat(TokenKind::Value).is_some() {
            SessionSetTarget::ValueParameter(self.parse_session_set_value_parameter()?)
        } else {
            return Err(self.error_expected(
                vec![
                    TokenKind::Schema,
                    TokenKind::Graph,
                    TokenKind::Property,
                    TokenKind::Binding,
                    TokenKind::Table,
                    TokenKind::Time,
                    TokenKind::Value,
                ],
                self.peek_kind(),
            ));
        };
        Ok(SessionSetCommand { target })
    }

    fn starts_session_set_graph_parameter(&self) -> bool {
        let mut trial = self.clone();
        trial
            .parse_session_set_graph_parameter(false)
            .is_ok_and(|_| is_statement_follow(trial.peek_kind()))
    }

    fn parse_session_set_graph_parameter(
        &mut self,
        property_graph: bool,
    ) -> Result<SessionSetGraphParameter> {
        let if_not_exists = self.parse_if_not_exists()?;
        let name = self.parse_parameter_name()?;
        let (typed, value_type) = self.parse_optional_definition_value_type()?;
        self.expect(TokenKind::Eq)?;
        Ok(SessionSetGraphParameter {
            property_graph,
            if_not_exists,
            name,
            typed,
            value_type,
            initializer: self.parse_graph_expression()?,
        })
    }

    fn parse_session_set_binding_table_parameter(
        &mut self,
        binding: bool,
    ) -> Result<SessionSetBindingTableParameter> {
        let if_not_exists = self.parse_if_not_exists()?;
        let name = self.parse_parameter_name()?;
        let (typed, value_type) = self.parse_optional_definition_value_type()?;
        self.expect(TokenKind::Eq)?;
        Ok(SessionSetBindingTableParameter {
            binding,
            if_not_exists,
            name,
            typed,
            value_type,
            initializer: self.parse_binding_table_expression()?,
        })
    }

    fn parse_session_set_value_parameter(&mut self) -> Result<SessionSetValueParameter> {
        let if_not_exists = self.parse_if_not_exists()?;
        let name = self.parse_parameter_name()?;
        let (typed, value_type) = if self.at(TokenKind::Eq) {
            (false, None)
        } else {
            let typed = self.parse_optional_typed_marker()?;
            let value_type = self.parse_value_type()?;
            (typed, Some(value_type))
        };
        self.expect(TokenKind::Eq)?;
        let initializer = self.parse_expr()?;
        Ok(SessionSetValueParameter {
            if_not_exists,
            name,
            typed,
            value_type,
            initializer,
        })
    }

    fn parse_parameter_name(&mut self) -> Result<String> {
        if self.at(TokenKind::Parameter) {
            let token = self.bump();
            Ok(token.text.trim_start_matches('$').to_owned())
        } else {
            Ok(self.parse_identifier()?.value)
        }
    }

    fn parse_session_prefixed_command(&mut self) -> Result<Statement> {
        self.expect(TokenKind::Session)?;
        match self.peek_kind() {
            TokenKind::Set => Ok(Statement::SessionSet(self.parse_session_set_command()?)),
            TokenKind::Reset => Ok(Statement::SessionReset(self.parse_session_reset_command()?)),
            TokenKind::Close => {
                self.bump();
                Ok(Statement::SessionClose(SessionCloseCommand))
            }
            kind => Err(self.error_expected(
                vec![TokenKind::Set, TokenKind::Reset, TokenKind::Close],
                kind,
            )),
        }
    }

    fn parse_session_reset_command(&mut self) -> Result<SessionResetCommand> {
        self.expect(TokenKind::Reset)?;
        let target = if matches!(self.peek_kind(), TokenKind::Semicolon | TokenKind::Eof) {
            SessionResetTarget::AllCharacteristics
        } else if self.eat(TokenKind::All).is_some() {
            if self.eat(TokenKind::Parameters).is_some() {
                SessionResetTarget::AllParameters
            } else if self.eat(TokenKind::Characteristics).is_some() {
                SessionResetTarget::AllCharacteristics
            } else {
                return Err(self.error_expected(
                    vec![TokenKind::Parameters, TokenKind::Characteristics],
                    self.peek_kind(),
                ));
            }
        } else if self.eat(TokenKind::Parameters).is_some() {
            SessionResetTarget::AllParameters
        } else if self.eat(TokenKind::Characteristics).is_some() {
            SessionResetTarget::AllCharacteristics
        } else if self.eat(TokenKind::Schema).is_some() {
            SessionResetTarget::Schema
        } else if self.eat(TokenKind::Property).is_some() {
            self.expect(TokenKind::Graph)?;
            SessionResetTarget::Graph
        } else if self.eat(TokenKind::Graph).is_some() {
            SessionResetTarget::Graph
        } else if self.eat(TokenKind::Time).is_some() {
            self.expect(TokenKind::Zone)?;
            SessionResetTarget::TimeZone
        } else if self.eat(TokenKind::ParameterKeyword).is_some()
            || matches!(self.peek_kind(), TokenKind::Parameter)
        {
            SessionResetTarget::Parameter(self.parse_parameter_name()?)
        } else {
            return Err(self.error_expected(
                vec![
                    TokenKind::All,
                    TokenKind::Parameters,
                    TokenKind::Characteristics,
                    TokenKind::Schema,
                    TokenKind::Property,
                    TokenKind::Graph,
                    TokenKind::Time,
                    TokenKind::ParameterKeyword,
                    TokenKind::Parameter,
                ],
                self.peek_kind(),
            ));
        };
        Ok(SessionResetCommand { target })
    }

    fn parse_start_transaction_statement(&mut self) -> Result<StartTransactionStatement> {
        self.expect(TokenKind::Start)?;
        self.expect(TokenKind::Transaction)?;
        let mut access_mode = None;

        if self.at(TokenKind::Isolation) {
            return Err(Error::Message {
                offset: self.peek().offset,
                message: "ISOLATION LEVEL is not a GQL transaction characteristic".to_owned(),
            });
        }

        if self.eat(TokenKind::Read).is_some() {
            access_mode = if self.eat(TokenKind::Only).is_some() {
                Some(TransactionAccessMode::ReadOnly)
            } else {
                self.expect(TokenKind::Write)?;
                Some(TransactionAccessMode::ReadWrite)
            };
            if self.eat(TokenKind::Comma).is_some() {
                if self.at(TokenKind::Read) {
                    return Err(Error::Message {
                        offset: self.peek().offset,
                        message: "transaction characteristics must contain exactly one access mode"
                            .to_owned(),
                    });
                }
                if self.at(TokenKind::Isolation) {
                    return Err(Error::Message {
                        offset: self.peek().offset,
                        message: "ISOLATION LEVEL is not a GQL transaction characteristic"
                            .to_owned(),
                    });
                }
                return Err(Error::Message {
                    offset: self.peek().offset,
                    message: "implementation-defined transaction modes are not supported"
                        .to_owned(),
                });
            }
        }

        Ok(StartTransactionStatement {
            access_mode,
            isolation_level: None,
        })
    }

    fn parse_commit_statement(&mut self) -> Result<CommitStatement> {
        self.expect(TokenKind::Commit)?;
        Ok(CommitStatement)
    }

    fn parse_rollback_statement(&mut self) -> Result<RollbackStatement> {
        self.expect(TokenKind::Rollback)?;
        Ok(RollbackStatement)
    }

    fn parse_delete_statement(&mut self) -> Result<DeleteStatement> {
        let detach = if self.eat(TokenKind::Detach).is_some() {
            self.expect(TokenKind::Delete)?;
            true
        } else {
            self.eat(TokenKind::Nodetach);
            self.expect(TokenKind::Delete)?;
            false
        };
        let mut items = Vec::new();
        loop {
            items.push(self.parse_expr()?);
            if self.eat(TokenKind::Comma).is_none() {
                break;
            }
        }
        Ok(DeleteStatement { detach, items })
    }

    fn parse_set_statement(&mut self) -> Result<SetStatement> {
        self.expect(TokenKind::Set)?;
        let mut items = Vec::new();
        loop {
            let offset = self.peek().offset;
            let item = self.parse_set_item()?;
            validate_set_item_conflicts(&items, &item, offset)?;
            items.push(item);
            if self.eat(TokenKind::Comma).is_none() {
                break;
            }
        }
        Ok(SetStatement { items })
    }

    fn parse_set_item(&mut self) -> Result<SetItem> {
        let variable = self.parse_identifier()?;
        if self.eat(TokenKind::Dot).is_some() {
            let property = self.parse_identifier()?;
            self.expect(TokenKind::Eq)?;
            return Ok(SetItem::Property {
                target: Expr::Property {
                    base: Box::new(Expr::Identifier(variable)),
                    key: property,
                },
                value: self.parse_expr()?,
            });
        }
        if self.eat(TokenKind::Eq).is_some() {
            return Ok(SetItem::AllProperties {
                variable,
                properties: self.parse_map_literal()?,
            });
        }
        if self.eat(TokenKind::Colon).is_some() || self.eat(TokenKind::Is).is_some() {
            return Ok(SetItem::Label {
                variable,
                label: self.parse_identifier()?,
            });
        }
        Err(self.error_expected(
            vec![
                TokenKind::Dot,
                TokenKind::Eq,
                TokenKind::Colon,
                TokenKind::Is,
            ],
            self.peek_kind(),
        ))
    }

    fn parse_remove_statement(&mut self) -> Result<RemoveStatement> {
        self.expect(TokenKind::Remove)?;
        let mut items = Vec::new();
        loop {
            let variable = self.parse_identifier()?;
            let item = if self.eat(TokenKind::Dot).is_some() {
                let property = self.parse_identifier()?;
                RemoveItem::Property(Expr::Property {
                    base: Box::new(Expr::Identifier(variable)),
                    key: property,
                })
            } else if self.eat(TokenKind::Colon).is_some() || self.eat(TokenKind::Is).is_some() {
                RemoveItem::Label {
                    variable,
                    label: self.parse_identifier()?,
                }
            } else {
                return Err(self.error_expected(
                    vec![TokenKind::Dot, TokenKind::Colon, TokenKind::Is],
                    self.peek_kind(),
                ));
            };
            items.push(item);
            if self.eat(TokenKind::Comma).is_none() {
                break;
            }
        }
        Ok(RemoveStatement { items })
    }

    fn parse_create_statement(&mut self) -> Result<Statement> {
        self.expect(TokenKind::Create)?;
        let or_replace = if self.eat(TokenKind::Or).is_some() {
            self.expect(TokenKind::Replace)?;
            true
        } else {
            false
        };
        let property_graph = self.eat(TokenKind::Property).is_some();
        match self.peek_kind() {
            TokenKind::Graph => {
                self.bump();
                if self.eat(TokenKind::Type).is_some() {
                    let if_not_exists = if or_replace {
                        false
                    } else {
                        self.parse_if_not_exists()?
                    };
                    let name = self.parse_graph_type_name()?;
                    let source = self.parse_create_graph_type_source()?;
                    Ok(Statement::CreateGraphType(CreateGraphTypeStatement {
                        if_not_exists,
                        or_replace,
                        property_graph,
                        name,
                        source,
                    }))
                } else {
                    let if_not_exists = if or_replace {
                        false
                    } else {
                        self.parse_if_not_exists()?
                    };
                    let name = self.parse_graph_name()?;
                    let graph_type = Some(self.parse_required_create_graph_type()?);
                    let source = self.parse_optional_graph_source()?;
                    Ok(Statement::CreateGraph(CreateGraphStatement {
                        if_not_exists,
                        or_replace,
                        property_graph,
                        name,
                        graph_type,
                        source,
                    }))
                }
            }
            TokenKind::Schema => {
                if or_replace || property_graph {
                    return Err(self.error_expected(vec![TokenKind::Graph], self.peek_kind()));
                }
                self.bump();
                let if_not_exists = self.parse_if_not_exists()?;
                let name = self.parse_schema_name()?;
                Ok(Statement::CreateSchema(CreateSchemaStatement {
                    if_not_exists,
                    name,
                }))
            }
            kind => Err(self.error_expected(vec![TokenKind::Graph, TokenKind::Schema], kind)),
        }
    }

    fn parse_drop_statement(&mut self) -> Result<Statement> {
        self.expect(TokenKind::Drop)?;
        let property_graph = self.eat(TokenKind::Property).is_some();
        match self.peek_kind() {
            TokenKind::Graph => {
                self.bump();
                if self.eat(TokenKind::Type).is_some() {
                    let if_exists = self.parse_if_exists()?;
                    let name = self.parse_graph_type_name()?;
                    Ok(Statement::DropGraphType(DropGraphTypeStatement {
                        if_exists,
                        property_graph,
                        name,
                    }))
                } else {
                    let if_exists = self.parse_if_exists()?;
                    let name = self.parse_graph_name()?;
                    Ok(Statement::DropGraph(DropGraphStatement {
                        if_exists,
                        property_graph,
                        name,
                    }))
                }
            }
            TokenKind::Schema => {
                if property_graph {
                    return Err(self.error_expected(vec![TokenKind::Graph], self.peek_kind()));
                }
                self.bump();
                let if_exists = self.parse_if_exists()?;
                let name = self.parse_schema_name()?;
                Ok(Statement::DropSchema(DropSchemaStatement {
                    if_exists,
                    name,
                }))
            }
            kind => Err(self.error_expected(vec![TokenKind::Graph, TokenKind::Schema], kind)),
        }
    }

    fn parse_if_not_exists(&mut self) -> Result<bool> {
        if self.eat(TokenKind::If).is_some() {
            self.expect(TokenKind::Not)?;
            self.expect(TokenKind::Exists)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn parse_if_exists(&mut self) -> Result<bool> {
        if self.eat(TokenKind::If).is_some() {
            self.expect(TokenKind::Exists)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn parse_graph_name(&mut self) -> Result<GraphName> {
        let (absolute, current_schema, home_schema, parent_levels, parts) =
            self.parse_catalog_name_parts()?;
        Ok(GraphName {
            absolute,
            current_schema,
            home_schema,
            parent_levels,
            parts,
        })
    }

    fn parse_graph_expression(&mut self) -> Result<GraphExpression> {
        if self.eat(TokenKind::Variable).is_some() {
            return Ok(GraphExpression::Variable(Box::new(self.parse_expr()?)));
        }
        if self.at(TokenKind::LParen) {
            return Ok(GraphExpression::Variable(Box::new(self.parse_expr()?)));
        }
        if self.starts_parameter_property_reference() {
            return Ok(GraphExpression::Variable(Box::new(self.parse_expr()?)));
        }
        if self.at(TokenKind::Parameter) {
            let token = self.bump();
            return Ok(GraphExpression::Parameter(
                token.text.trim_start_matches('$').to_owned(),
            ));
        }
        if self.eat(TokenKind::CurrentGraph).is_some() {
            return Ok(GraphExpression::CurrentGraph);
        }
        if self.eat(TokenKind::CurrentPropertyGraph).is_some() {
            return Ok(GraphExpression::CurrentPropertyGraph);
        }
        if self.eat(TokenKind::HomeGraph).is_some() {
            return Ok(GraphExpression::HomeGraph);
        }
        if self.eat(TokenKind::HomePropertyGraph).is_some() {
            return Ok(GraphExpression::HomePropertyGraph);
        }
        if self.starts_object_expression_primary_special_case(self.peek_kind()) {
            return Ok(GraphExpression::Variable(Box::new(self.parse_expr()?)));
        }
        Ok(GraphExpression::Name(self.parse_graph_name()?))
    }

    fn parse_binding_table_name(&mut self) -> Result<BindingTableName> {
        let (absolute, current_schema, home_schema, parent_levels, parts) =
            self.parse_catalog_name_parts()?;
        Ok(BindingTableName {
            absolute,
            current_schema,
            home_schema,
            parent_levels,
            parts,
        })
    }

    fn parse_binding_table_expression(&mut self) -> Result<BindingTableExpression> {
        if self.at(TokenKind::LBrace) {
            let mut trial = self.clone();
            if let Ok(query) = trial.parse_nested_query_specification() {
                *self = trial;
                return Ok(BindingTableExpression::NestedQuery(Box::new(query)));
            }
        }
        if self.eat(TokenKind::Variable).is_some() {
            return Ok(BindingTableExpression::Variable(Box::new(
                self.parse_expr()?,
            )));
        }
        if self.at(TokenKind::LParen) {
            return Ok(BindingTableExpression::Variable(Box::new(
                self.parse_expr()?,
            )));
        }
        if self.starts_parameter_property_reference() {
            return Ok(BindingTableExpression::Variable(Box::new(
                self.parse_expr()?,
            )));
        }
        if self.at(TokenKind::Parameter) {
            let token = self.bump();
            return Ok(BindingTableExpression::Parameter(
                token.text.trim_start_matches('$').to_owned(),
            ));
        }
        if self.starts_object_expression_primary_special_case(self.peek_kind()) {
            return Ok(BindingTableExpression::Variable(Box::new(
                self.parse_expr()?,
            )));
        }
        Ok(BindingTableExpression::Name(
            self.parse_binding_table_name()?,
        ))
    }

    fn parse_graph_type_name(&mut self) -> Result<GraphTypeName> {
        let (absolute, current_schema, home_schema, parent_levels, parts) =
            self.parse_catalog_name_parts()?;
        Ok(GraphTypeName {
            absolute,
            current_schema,
            home_schema,
            parent_levels,
            parts,
        })
    }

    fn parse_schema_name(&mut self) -> Result<SchemaName> {
        let (absolute, current_schema, home_schema, parent_levels, parts) =
            self.parse_catalog_name_parts()?;
        Ok(SchemaName {
            absolute,
            current_schema,
            home_schema,
            parent_levels,
            parts,
        })
    }

    fn parse_schema_reference(&mut self) -> Result<SchemaReference> {
        if self.at(TokenKind::Parameter) {
            let token = self.bump();
            return Ok(SchemaReference::Parameter(
                token.text.trim_start_matches('$').to_owned(),
            ));
        }
        if self.eat(TokenKind::CurrentSchema).is_some() {
            return Ok(SchemaReference::CurrentSchema);
        }
        if self.eat(TokenKind::HomeSchema).is_some() {
            return Ok(SchemaReference::HomeSchema);
        }
        if self.eat(TokenKind::Slash).is_some() {
            if is_schema_reference_follow(self.peek_kind()) || !is_identifier_like(self.peek_kind())
            {
                return Ok(SchemaReference::Root);
            }
            return Ok(SchemaReference::Absolute(self.parse_schema_name()?));
        }
        if self.eat(TokenKind::Dot).is_some() {
            if self.eat(TokenKind::Dot).is_none() {
                return Ok(SchemaReference::CurrentSchema);
            }
            let mut levels = 1;
            loop {
                self.expect(TokenKind::Slash)?;
                if self.eat(TokenKind::Dot).is_some() {
                    self.expect(TokenKind::Dot)?;
                    levels += 1;
                } else {
                    let name = self.parse_schema_name()?;
                    return Ok(SchemaReference::Parent { levels, name });
                }
            }
        }
        Ok(SchemaReference::Name(self.parse_schema_name()?))
    }

    fn parse_procedure_name(&mut self) -> Result<ProcedureName> {
        let (absolute, current_schema, home_schema, parent_levels, parts) =
            self.parse_catalog_name_parts()?;
        Ok(ProcedureName {
            absolute,
            current_schema,
            home_schema,
            parent_levels,
            parts,
        })
    }

    fn parse_procedure_reference(&mut self) -> Result<ProcedureReference> {
        if self.at(TokenKind::Parameter) {
            let token = self.bump();
            return Ok(ProcedureReference::Parameter(
                token.text.trim_start_matches('$').to_owned(),
            ));
        }
        Ok(ProcedureReference::Name(self.parse_procedure_name()?))
    }

    fn parse_catalog_name_parts(&mut self) -> Result<(bool, bool, bool, usize, Vec<Identifier>)> {
        let absolute = self.eat(TokenKind::Slash).is_some();
        let mut current_schema = false;
        let mut home_schema = false;
        let mut parent_levels = 0;

        if !absolute {
            if self.eat(TokenKind::CurrentSchema).is_some() {
                self.expect(TokenKind::Slash)?;
                current_schema = true;
            } else if self.eat(TokenKind::HomeSchema).is_some() {
                self.expect(TokenKind::Slash)?;
                home_schema = true;
            } else if self.eat(TokenKind::Dot).is_some() {
                if self.eat(TokenKind::Dot).is_some() {
                    parent_levels = 1;
                    loop {
                        self.expect(TokenKind::Slash)?;
                        if self.eat(TokenKind::Dot).is_some() {
                            self.expect(TokenKind::Dot)?;
                            parent_levels += 1;
                        } else {
                            break;
                        }
                    }
                } else {
                    self.expect(TokenKind::Slash)?;
                    current_schema = true;
                }
            }
        }

        let mut parts = vec![self.parse_identifier()?];
        while matches!(self.peek_kind(), TokenKind::Dot | TokenKind::Slash) {
            self.bump();
            parts.push(self.parse_identifier()?);
        }
        Ok((absolute, current_schema, home_schema, parent_levels, parts))
    }

    fn starts_parameter_property_reference(&self) -> bool {
        self.at(TokenKind::Parameter) && self.peek_n_kind(1) == Some(TokenKind::Dot)
    }

    fn parse_required_create_graph_type(&mut self) -> Result<CreateGraphType> {
        if let Some(graph_type) = self.parse_optional_create_graph_type()? {
            return Ok(graph_type);
        }

        Err(self.error_expected(
            vec![
                TokenKind::Any,
                TokenKind::Like,
                TokenKind::Typed,
                TokenKind::DoubleColon,
                TokenKind::Property,
                TokenKind::Graph,
                TokenKind::LBrace,
                TokenKind::Parameter,
                TokenKind::Slash,
                TokenKind::Dot,
                TokenKind::Identifier,
            ],
            self.peek_kind(),
        ))
    }

    fn parse_optional_create_graph_type(&mut self) -> Result<Option<CreateGraphType>> {
        if matches!(
            self.peek_kind(),
            TokenKind::As | TokenKind::Semicolon | TokenKind::Eof
        ) {
            return Ok(None);
        }

        if self.eat(TokenKind::Like).is_some() {
            return Ok(Some(CreateGraphType::Like(self.parse_graph_expression()?)));
        }

        let typed = self.parse_optional_typed_marker()?;

        if self.eat(TokenKind::Any).is_some() {
            let property_graph = self.parse_optional_property_graph_prefix()?;
            return Ok(Some(CreateGraphType::Any {
                typed,
                property_graph,
            }));
        }

        if self.at(TokenKind::Property) || self.at(TokenKind::Graph) || self.at(TokenKind::LBrace) {
            let property_graph = self.parse_optional_property_graph_prefix()?;
            self.eat(TokenKind::Type);
            let body = self.parse_nested_graph_type_body()?;
            return Ok(Some(CreateGraphType::Nested {
                typed,
                property_graph,
                body,
            }));
        }

        if typed
            || matches!(self.peek_kind(), TokenKind::Parameter)
            || matches!(self.peek_kind(), TokenKind::Slash)
            || matches!(self.peek_kind(), TokenKind::Dot)
            || is_identifier_like(self.peek_kind())
        {
            let name = self.parse_graph_type_reference()?;
            return Ok(Some(CreateGraphType::Named { typed, name }));
        }

        Ok(None)
    }

    fn parse_optional_graph_source(&mut self) -> Result<Option<GraphExpression>> {
        if self.eat(TokenKind::As).is_none() {
            return Ok(None);
        }
        self.expect(TokenKind::Copy)?;
        self.expect(TokenKind::Of)?;
        Ok(Some(self.parse_graph_expression()?))
    }

    fn parse_optional_typed_marker(&mut self) -> Result<bool> {
        if self.eat(TokenKind::DoubleColon).is_some() {
            self.eat(TokenKind::Typed);
            return Ok(true);
        }
        if self.eat(TokenKind::Typed).is_some() {
            return Ok(true);
        }
        Ok(false)
    }

    fn parse_optional_property_graph_prefix(&mut self) -> Result<bool> {
        if self.eat(TokenKind::Property).is_some() {
            self.expect(TokenKind::Graph)?;
            return Ok(true);
        }
        self.eat(TokenKind::Graph);
        Ok(false)
    }

    fn parse_create_graph_type_source(&mut self) -> Result<CreateGraphTypeSource> {
        if self.eat(TokenKind::As).is_some() {
            if self.eat(TokenKind::Copy).is_some() {
                self.expect(TokenKind::Of)?;
                return self.parse_copy_of_graph_type_source();
            }

            return Ok(CreateGraphTypeSource::Nested(
                self.parse_nested_graph_type_body()?,
            ));
        }

        if self.eat(TokenKind::Copy).is_some() {
            self.expect(TokenKind::Of)?;
            return self.parse_copy_of_graph_type_source();
        }

        if self.eat(TokenKind::Like).is_some() {
            return Ok(CreateGraphTypeSource::Like(self.parse_graph_expression()?));
        }

        Ok(CreateGraphTypeSource::Nested(
            self.parse_nested_graph_type_body()?,
        ))
    }

    fn parse_copy_of_graph_type_source(&mut self) -> Result<CreateGraphTypeSource> {
        if self.at(TokenKind::String) {
            let token = self.bump();
            if !token.text.contains(':') {
                return Err(Error::Message {
                    offset: token.offset,
                    message: "external object reference must contain ':'".to_owned(),
                });
            }
            return Ok(CreateGraphTypeSource::CopyOfExternal(token.text));
        }
        Ok(CreateGraphTypeSource::CopyOf(
            self.parse_graph_type_reference()?,
        ))
    }

    fn parse_graph_type_reference(&mut self) -> Result<GraphTypeReference> {
        if self.at(TokenKind::Parameter) {
            let token = self.bump();
            return Ok(GraphTypeReference::Parameter(
                token.text.trim_start_matches('$').to_owned(),
            ));
        }
        Ok(GraphTypeReference::Name(self.parse_graph_type_name()?))
    }

    fn parse_nested_graph_type_body(&mut self) -> Result<GraphTypeBody> {
        self.expect(TokenKind::LBrace)?;
        let mut elements = Vec::new();
        if !self.at(TokenKind::RBrace) {
            loop {
                let offset = self.peek().offset;
                let element = self.parse_graph_type_element(
                    GraphTypeElementContext::GraphTypeSpecificationBody,
                )?;
                validate_graph_type_element_name(&elements, &element, offset)?;
                elements.push(element);
                if self.eat(TokenKind::Comma).is_none() {
                    break;
                }
            }
        }
        validate_graph_type_endpoint_node_types(&elements, self.peek().offset)?;
        self.expect(TokenKind::RBrace)?;
        Ok(GraphTypeBody { elements })
    }

    fn parse_graph_type_element(
        &mut self,
        context: GraphTypeElementContext,
    ) -> Result<GraphTypeElement> {
        let offset = self.peek().offset;
        let edge_kind = self.parse_optional_edge_kind_before_synonym();
        if self.eat_node_synonym() {
            let name = if self.starts_node_type_filler() {
                Identifier::new("")
            } else {
                self.parse_type_phrase_name()?
            };
            let labels = self.parse_optional_type_label_list()?;
            let properties = self.parse_optional_property_type_set()?;
            Ok(GraphTypeElement::Node(NodeTypeDefinition {
                name,
                labels,
                properties,
            }))
        } else if edge_kind.is_some() || self.eat_edge_synonym() {
            if edge_kind.is_some() {
                self.expect_edge_synonym()?;
            }
            let name = if self.starts_node_type_filler() {
                Identifier::new("")
            } else {
                self.parse_type_phrase_name()?
            };
            validate_graph_type_body_edge_phrase(context, edge_kind, &name, offset)?;
            let labels = self.parse_optional_type_label_list()?;
            let (direction, source, destination, properties) =
                self.parse_edge_type_tail(edge_kind.unwrap_or(Direction::Right), edge_kind)?;
            Ok(GraphTypeElement::Edge(EdgeTypeDefinition {
                name,
                direction,
                labels,
                source,
                destination,
                properties,
            }))
        } else if self.eat(TokenKind::LParen).is_some() {
            let name = if self.at(TokenKind::RParen) || self.starts_node_type_filler() {
                None
            } else {
                Some(self.parse_identifier()?)
            };
            if self.eat(TokenKind::RParen).is_some() {
                if self.starts_full_edge_type_arc() {
                    return self.parse_full_edge_type_pattern_after_left_node(name);
                }
                return Ok(GraphTypeElement::Node(NodeTypeDefinition {
                    name: name.unwrap_or_else(|| Identifier::new("")),
                    labels: Vec::new(),
                    properties: Vec::new(),
                }));
            }
            let labels = self.parse_optional_type_label_list()?;
            let properties = self.parse_optional_property_type_set()?;
            self.expect(TokenKind::RParen)?;
            if self.starts_full_edge_type_arc() {
                return self.parse_full_edge_type_pattern_after_left_node(name);
            }
            Ok(GraphTypeElement::Node(NodeTypeDefinition {
                name: name.unwrap_or_else(|| Identifier::new("")),
                labels,
                properties,
            }))
        } else {
            Err(self.error_expected(
                vec![
                    TokenKind::Node,
                    TokenKind::Vertex,
                    TokenKind::Edge,
                    TokenKind::Relationship,
                    TokenKind::LParen,
                ],
                self.peek_kind(),
            ))
        }
    }

    fn eat_node_synonym(&mut self) -> bool {
        self.eat(TokenKind::Node).is_some() || self.eat(TokenKind::Vertex).is_some()
    }

    fn eat_edge_synonym(&mut self) -> bool {
        self.eat(TokenKind::Edge).is_some() || self.eat(TokenKind::Relationship).is_some()
    }

    fn expect_edge_synonym(&mut self) -> Result<()> {
        if self.eat_edge_synonym() {
            Ok(())
        } else {
            Err(self.error_expected(
                vec![TokenKind::Edge, TokenKind::Relationship],
                self.peek_kind(),
            ))
        }
    }

    fn at_node_synonym(&self) -> bool {
        matches!(self.peek_kind(), TokenKind::Node | TokenKind::Vertex)
    }

    fn at_edge_synonym(&self) -> bool {
        matches!(self.peek_kind(), TokenKind::Edge | TokenKind::Relationship)
    }

    fn parse_optional_edge_kind_before_synonym(&mut self) -> Option<Direction> {
        if self
            .peek_n_kind(1)
            .is_none_or(|kind| !is_edge_synonym_kind(kind))
        {
            return None;
        }
        if self.eat(TokenKind::Directed).is_some() {
            Some(Direction::Right)
        } else if self.eat(TokenKind::Undirected).is_some() {
            Some(Direction::Undirected)
        } else {
            None
        }
    }

    fn at_edge_kind_before_synonym(&self) -> bool {
        matches!(
            self.peek_kind(),
            TokenKind::Directed | TokenKind::Undirected
        ) && self.peek_n_kind(1).is_some_and(is_edge_synonym_kind)
    }

    fn starts_node_type_filler(&self) -> bool {
        match self.peek_kind() {
            TokenKind::Label | TokenKind::Labels => {
                self.peek_n_kind(1).is_some_and(is_identifier_like)
            }
            TokenKind::Colon | TokenKind::Is | TokenKind::LBrace => true,
            _ => false,
        }
    }

    fn parse_type_phrase_name(&mut self) -> Result<Identifier> {
        if self.at(TokenKind::Type)
            && self
                .peek_n_kind(1)
                .is_some_and(is_type_name_after_type_marker)
        {
            self.bump();
        }
        self.parse_identifier()
    }

    fn starts_full_edge_type_arc(&self) -> bool {
        matches!(
            self.peek_kind(),
            TokenKind::Minus | TokenKind::ArrowRight | TokenKind::ArrowLeft | TokenKind::Tilde
        )
    }

    fn parse_full_edge_type_pattern_after_left_node(
        &mut self,
        left_node: Option<Identifier>,
    ) -> Result<GraphTypeElement> {
        if self.eat(TokenKind::ArrowRight).is_some() {
            let destination = self.parse_parenthesized_node_type_reference()?;
            return Ok(GraphTypeElement::Edge(EdgeTypeDefinition {
                name: Identifier::new(""),
                direction: Direction::Right,
                labels: Vec::new(),
                source: left_node,
                destination,
                properties: Vec::new(),
            }));
        }

        if self.eat(TokenKind::Minus).is_some() {
            self.expect(TokenKind::LBracket)?;
            let (name, labels, properties) = self.parse_named_arc_type_filler()?;
            self.expect(TokenKind::RBracket)?;
            self.expect(TokenKind::ArrowRight)?;
            let destination = self.parse_parenthesized_node_type_reference()?;
            return Ok(GraphTypeElement::Edge(EdgeTypeDefinition {
                name,
                direction: Direction::Right,
                labels,
                source: left_node,
                destination,
                properties,
            }));
        }

        if self.eat(TokenKind::Tilde).is_some() {
            if self.at(TokenKind::LParen) {
                let destination = self.parse_parenthesized_node_type_reference()?;
                return Ok(GraphTypeElement::Edge(EdgeTypeDefinition {
                    name: Identifier::new(""),
                    direction: Direction::Undirected,
                    labels: Vec::new(),
                    source: left_node,
                    destination,
                    properties: Vec::new(),
                }));
            }
            self.expect(TokenKind::LBracket)?;
            let (name, labels, properties) = self.parse_named_arc_type_filler()?;
            self.expect(TokenKind::RBracket)?;
            self.expect(TokenKind::Tilde)?;
            let destination = self.parse_parenthesized_node_type_reference()?;
            return Ok(GraphTypeElement::Edge(EdgeTypeDefinition {
                name,
                direction: Direction::Undirected,
                labels,
                source: left_node,
                destination,
                properties,
            }));
        }

        self.expect(TokenKind::ArrowLeft)?;
        if self.at(TokenKind::LParen) {
            let source = self.parse_parenthesized_node_type_reference()?;
            return Ok(GraphTypeElement::Edge(EdgeTypeDefinition {
                name: Identifier::new(""),
                direction: Direction::Right,
                labels: Vec::new(),
                source,
                destination: left_node,
                properties: Vec::new(),
            }));
        }
        self.expect(TokenKind::LBracket)?;
        let (name, labels, properties) = self.parse_named_arc_type_filler()?;
        self.expect(TokenKind::RBracket)?;
        self.expect(TokenKind::Minus)?;
        let source = self.parse_parenthesized_node_type_reference()?;
        Ok(GraphTypeElement::Edge(EdgeTypeDefinition {
            name,
            direction: Direction::Right,
            labels,
            source,
            destination: left_node,
            properties,
        }))
    }

    fn parse_named_arc_type_filler(
        &mut self,
    ) -> Result<(Identifier, Vec<Identifier>, Vec<PropertyTypeDefinition>)> {
        let name = if self.at(TokenKind::RBracket) || self.starts_node_type_filler() {
            Identifier::new("")
        } else {
            self.parse_identifier()?
        };
        let labels = self.parse_optional_type_label_list()?;
        let properties = self.parse_optional_property_type_set()?;
        Ok((name, labels, properties))
    }

    fn parse_edge_type_tail(
        &mut self,
        default_direction: Direction,
        explicit_edge_kind: Option<Direction>,
    ) -> Result<EdgeTypeTail> {
        let mut properties = self.parse_optional_property_type_set()?;
        let (direction, source, destination) = if self.eat(TokenKind::Connecting).is_some() {
            let (direction, source, destination) =
                self.parse_endpoint_pair_definition(default_direction, explicit_edge_kind)?;
            (direction, source, destination)
        } else {
            let source = if self.eat(TokenKind::From).is_some() {
                Some(self.parse_identifier()?)
            } else {
                None
            };
            let destination = if self.eat(TokenKind::To).is_some() {
                Some(self.parse_identifier()?)
            } else {
                None
            };
            (default_direction, source, destination)
        };
        if properties.is_empty() {
            properties = self.parse_optional_property_type_set()?;
        }
        Ok((direction, source, destination, properties))
    }

    fn parse_endpoint_pair_definition(
        &mut self,
        default_direction: Direction,
        explicit_edge_kind: Option<Direction>,
    ) -> Result<(Direction, Option<Identifier>, Option<Identifier>)> {
        self.expect(TokenKind::LParen)?;
        let first = self.parse_identifier()?;
        if self.eat(TokenKind::RParen).is_some() {
            let left_node = Some(first);
            if self.at(TokenKind::ArrowRight) {
                let offset = self.bump().offset;
                validate_endpoint_pair_shape(explicit_edge_kind, Direction::Right, offset)?;
                return Ok((
                    Direction::Right,
                    left_node,
                    self.parse_parenthesized_node_type_reference()?,
                ));
            }
            if self.at(TokenKind::ArrowLeft) {
                let offset = self.bump().offset;
                validate_endpoint_pair_shape(explicit_edge_kind, Direction::Right, offset)?;
                return Ok((
                    Direction::Right,
                    self.parse_parenthesized_node_type_reference()?,
                    left_node,
                ));
            }
            if self.at(TokenKind::Tilde) {
                let offset = self.bump().offset;
                validate_endpoint_pair_shape(explicit_edge_kind, Direction::Undirected, offset)?;
                return Ok((
                    Direction::Undirected,
                    left_node,
                    self.parse_parenthesized_node_type_reference()?,
                ));
            }
            return Err(self.error_expected(
                vec![
                    TokenKind::To,
                    TokenKind::ArrowRight,
                    TokenKind::ArrowLeft,
                    TokenKind::Tilde,
                ],
                self.peek_kind(),
            ));
        }
        let (direction, source, destination) = if self.eat(TokenKind::To).is_some() {
            (
                default_direction,
                Some(first),
                Some(self.parse_identifier()?),
            )
        } else if self.at(TokenKind::ArrowRight) {
            let offset = self.bump().offset;
            validate_endpoint_pair_shape(explicit_edge_kind, Direction::Right, offset)?;
            (
                Direction::Right,
                Some(first),
                Some(self.parse_identifier()?),
            )
        } else if self.at(TokenKind::Tilde) {
            let offset = self.bump().offset;
            validate_endpoint_pair_shape(explicit_edge_kind, Direction::Undirected, offset)?;
            (
                Direction::Undirected,
                Some(first),
                Some(self.parse_identifier()?),
            )
        } else if self.at(TokenKind::ArrowLeft) {
            let offset = self.bump().offset;
            validate_endpoint_pair_shape(explicit_edge_kind, Direction::Right, offset)?;
            (
                Direction::Right,
                Some(self.parse_identifier()?),
                Some(first),
            )
        } else {
            return Err(self.error_expected(
                vec![
                    TokenKind::To,
                    TokenKind::ArrowRight,
                    TokenKind::ArrowLeft,
                    TokenKind::Tilde,
                ],
                self.peek_kind(),
            ));
        };
        self.expect(TokenKind::RParen)?;
        Ok((direction, source, destination))
    }

    fn parse_parenthesized_node_type_reference(&mut self) -> Result<Option<Identifier>> {
        self.expect(TokenKind::LParen)?;
        let name = if self.at(TokenKind::RParen) || self.starts_node_type_filler() {
            None
        } else {
            Some(self.parse_identifier()?)
        };
        if name.is_none() {
            self.parse_optional_type_label_list()?;
            self.parse_optional_property_type_set()?;
        }
        self.expect(TokenKind::RParen)?;
        Ok(name)
    }

    fn parse_optional_type_label_list(&mut self) -> Result<Vec<Identifier>> {
        let mut labels = Vec::new();
        if self.eat(TokenKind::Label).is_some() {
            let label = self.parse_identifier()?;
            push_identifier_if_absent(&mut labels, Some(&label));
            return Ok(labels);
        }
        if self.eat(TokenKind::Labels).is_some() {
            let label = self.parse_identifier()?;
            push_identifier_if_absent(&mut labels, Some(&label));
            while self.eat(TokenKind::Ampersand).is_some() {
                let label = self.parse_identifier()?;
                push_identifier_if_absent(&mut labels, Some(&label));
            }
            return Ok(labels);
        }
        while self.eat(TokenKind::Colon).is_some() || self.eat(TokenKind::Is).is_some() {
            let label = self.parse_identifier()?;
            push_identifier_if_absent(&mut labels, Some(&label));
            while self.eat(TokenKind::Ampersand).is_some() {
                let label = self.parse_identifier()?;
                push_identifier_if_absent(&mut labels, Some(&label));
            }
        }
        Ok(labels)
    }

    fn parse_optional_property_type_set(&mut self) -> Result<Vec<PropertyTypeDefinition>> {
        let mut properties = Vec::new();
        if self.eat(TokenKind::LBrace).is_none() {
            return Ok(properties);
        }
        if !self.at(TokenKind::RBrace) {
            loop {
                let offset = self.peek().offset;
                let name = self.parse_identifier()?;
                if properties.iter().any(|property: &PropertyTypeDefinition| {
                    property.name.value.eq_ignore_ascii_case(&name.value)
                }) {
                    return Err(Error::Message {
                        offset,
                        message: format!("duplicate property '{}'", name.value),
                    });
                }
                self.parse_optional_typed_marker()?;
                let value_type = self.parse_value_type()?;
                properties.push(PropertyTypeDefinition { name, value_type });
                if self.eat(TokenKind::Comma).is_none() {
                    break;
                }
            }
        }
        self.expect(TokenKind::RBrace)?;
        Ok(properties)
    }

    fn parse_value_type(&mut self) -> Result<ValueType> {
        let first = self.parse_value_type_component()?;
        if self.eat(TokenKind::Pipe).is_none() {
            return Ok(first);
        }

        let mut components = vec![first, self.parse_value_type_component()?];
        while self.eat(TokenKind::Pipe).is_some() {
            components.push(self.parse_value_type_component()?);
        }
        validate_dynamic_union_component_nullability(&components, self.peek().offset)?;
        Ok(ValueType::DynamicUnion(components))
    }

    fn parse_value_type_component(&mut self) -> Result<ValueType> {
        let mut value_type = self.parse_value_type_base()?;
        if self.eat(TokenKind::Not).is_some() {
            self.expect(TokenKind::Null)?;
            value_type = ValueType::NotNull(Box::new(value_type));
        }
        if self.at(TokenKind::Group)
            && matches!(
                self.peek_n_kind(1),
                Some(TokenKind::List | TokenKind::Array)
            )
        {
            return Err(Error::Message {
                offset: self.peek().offset,
                message: "GROUP list value type names are not user-visible standard GQL syntax"
                    .to_owned(),
            });
        }
        while self.peek_list_value_type_name().is_some() {
            let offset = self.peek().offset;
            let (kind, group) = self.parse_list_value_type_name()?;
            let max_length = self.parse_optional_list_max_length()?;
            value_type = Self::wrap_list_value_type(value_type, kind, group, max_length, offset)?;
            if self.eat(TokenKind::Not).is_some() {
                self.expect(TokenKind::Null)?;
                value_type = ValueType::NotNull(Box::new(value_type));
            }
            if self.at(TokenKind::Group)
                && matches!(
                    self.peek_n_kind(1),
                    Some(TokenKind::List | TokenKind::Array)
                )
            {
                return Err(Error::Message {
                    offset: self.peek().offset,
                    message: "GROUP list value type names are not user-visible standard GQL syntax"
                        .to_owned(),
                });
            }
        }
        Ok(value_type)
    }

    fn parse_value_type_base(&mut self) -> Result<ValueType> {
        if self.at(TokenKind::Group)
            && matches!(
                self.peek_n_kind(1),
                Some(TokenKind::List | TokenKind::Array)
            )
        {
            return Err(Error::Message {
                offset: self.peek().offset,
                message: "GROUP list value type names are not user-visible standard GQL syntax"
                    .to_owned(),
            });
        }

        if self.eat(TokenKind::Any).is_some() {
            if self.eat(TokenKind::Property).is_some() {
                if self.eat(TokenKind::Graph).is_some() {
                    return Ok(ValueType::GraphReference {
                        property_graph: true,
                        body: None,
                    });
                }
                self.expect(TokenKind::Value)?;
                return Ok(ValueType::PropertyValue);
            }
            if self.eat(TokenKind::Graph).is_some() {
                return Ok(ValueType::GraphReference {
                    property_graph: false,
                    body: None,
                });
            }
            if self.eat_node_synonym() {
                return Ok(ValueType::NodeReference { definition: None });
            }
            if self.eat_edge_synonym() {
                return Ok(ValueType::EdgeReference { definition: None });
            }
            if self.eat(TokenKind::Record).is_some() {
                return Ok(ValueType::AnyRecord);
            }
            self.eat(TokenKind::Value);
            if self.eat(TokenKind::Lt).is_some() {
                let components = self.parse_component_type_list()?;
                self.expect(TokenKind::Gt)?;
                return Ok(ValueType::DynamicUnion(components));
            }
            return Ok(ValueType::AnyDynamic);
        }

        if self.at(TokenKind::Property) && self.peek_n_kind(1) == Some(TokenKind::Value) {
            self.expect(TokenKind::Property)?;
            self.expect(TokenKind::Value)?;
            return Ok(ValueType::PropertyValue);
        }

        if self.at(TokenKind::Binding) && self.peek_n_kind(1) == Some(TokenKind::Table) {
            self.expect(TokenKind::Binding)?;
            self.expect(TokenKind::Table)?;
            let fields = self.parse_field_type_specification()?;
            return Ok(ValueType::BindingTable {
                binding: true,
                fields,
            });
        }

        if self.at(TokenKind::Table) && self.peek_n_kind(1) == Some(TokenKind::LBrace) {
            self.expect(TokenKind::Table)?;
            let fields = self.parse_field_type_specification()?;
            return Ok(ValueType::BindingTable {
                binding: false,
                fields,
            });
        }

        if self.at(TokenKind::Property) && self.peek_n_kind(1) == Some(TokenKind::Graph) {
            self.expect(TokenKind::Property)?;
            self.expect(TokenKind::Graph)?;
            let body = self.parse_nested_graph_type_body()?;
            return Ok(ValueType::GraphReference {
                property_graph: true,
                body: Some(body),
            });
        }

        if self.at(TokenKind::Graph) && self.peek_n_kind(1) == Some(TokenKind::LBrace) {
            self.expect(TokenKind::Graph)?;
            let body = self.parse_nested_graph_type_body()?;
            return Ok(ValueType::GraphReference {
                property_graph: false,
                body: Some(body),
            });
        }

        if self.at_node_synonym() {
            return Ok(ValueType::NodeReference {
                definition: self.parse_optional_node_type_definition()?,
            });
        }

        if self.at_edge_kind_before_synonym() {
            let offset = self.peek().offset;
            let direction = self
                .parse_optional_edge_kind_before_synonym()
                .unwrap_or(Direction::Right);
            let definition = self
                .parse_optional_edge_type_definition_with_direction(direction, Some(direction))?;
            if let Some(definition) = &definition {
                validate_closed_edge_reference_definition(definition, offset)?;
            }
            return Ok(ValueType::EdgeReference { definition });
        }

        if self.at_edge_synonym() {
            let offset = self.peek().offset;
            let definition = self.parse_optional_edge_type_definition()?;
            if let Some(definition) = &definition {
                validate_closed_edge_reference_definition(definition, offset)?;
            }
            return Ok(ValueType::EdgeReference { definition });
        }

        if self.at(TokenKind::LParen) {
            return match self
                .parse_graph_type_element(GraphTypeElementContext::ClosedReferenceValueType)?
            {
                GraphTypeElement::Node(definition) => Ok(ValueType::NodeReference {
                    definition: Some(definition),
                }),
                GraphTypeElement::Edge(definition) => {
                    validate_closed_edge_reference_definition(&definition, self.peek().offset)?;
                    Ok(ValueType::EdgeReference {
                        definition: Some(definition),
                    })
                }
            };
        }

        if self.at(TokenKind::Path) {
            self.bump();
            return Ok(ValueType::Path);
        }

        if self.peek_list_value_type_name().is_some() {
            let offset = self.peek().offset;
            let (kind, group) = self.parse_list_value_type_name()?;
            self.expect(TokenKind::Lt)?;
            let inner = self.parse_value_type()?;
            self.expect(TokenKind::Gt)?;
            let max_length = self.parse_optional_list_max_length()?;
            return Self::wrap_list_value_type(inner, kind, group, max_length, offset);
        }

        if self.eat(TokenKind::Record).is_some() {
            if self.at(TokenKind::LBrace) {
                return Ok(ValueType::Record(self.parse_field_type_specification()?));
            }
            return Ok(ValueType::AnyRecord);
        }

        if self.at(TokenKind::LBrace) {
            return Ok(ValueType::Record(self.parse_field_type_specification()?));
        }

        if let Some(kind) = self.peek_character_string_type_kind() {
            self.bump();
            return self.parse_character_string_type(kind);
        }

        if let Some(kind) = self.peek_boolean_type_kind() {
            self.bump();
            return Ok(ValueType::Boolean { kind });
        }

        if let Some((kind, word_count)) = self.peek_temporal_type_kind() {
            for _ in 0..word_count {
                self.bump();
            }
            return Ok(ValueType::Temporal { kind });
        }

        if let Some(kind) = self.peek_byte_string_type_kind() {
            self.bump();
            return self.parse_byte_string_type(kind);
        }

        if let Some(kind) = self.peek_exact_numeric_type_kind() {
            self.bump();
            return self.parse_exact_numeric_type(kind);
        }

        if self.at(TokenKind::Identifier)
            && matches!(
                self.peek().text.to_ascii_uppercase().as_str(),
                "SIGNED" | "UNSIGNED"
            )
        {
            return self.parse_signed_or_unsigned_integer_type();
        }

        if self.peek_integer_numeric_type_start() {
            return self.parse_integer_numeric_type(None);
        }

        if let Some(kind) = self.peek_approximate_numeric_type_kind() {
            let token = self.bump();
            return self.parse_approximate_numeric_type(kind, token.text);
        }

        let name = self.parse_value_type_name()?;
        let mut parameters = Vec::new();
        if self.eat(TokenKind::LParen).is_some() {
            if !self.at(TokenKind::RParen) {
                loop {
                    parameters.push(self.parse_u64_literal()?);
                    if self.eat(TokenKind::Comma).is_none() {
                        break;
                    }
                }
            }
            self.expect(TokenKind::RParen)?;
        }
        Ok(ValueType::Named { name, parameters })
    }

    fn peek_character_string_type_kind(&self) -> Option<CharacterStringTypeKind> {
        let text = self.peek().text.to_ascii_uppercase();
        match text.as_str() {
            "STRING" => Some(CharacterStringTypeKind::String),
            "VARCHAR" => Some(CharacterStringTypeKind::Varchar),
            _ => None,
        }
    }

    fn parse_character_string_type(&mut self, kind: CharacterStringTypeKind) -> Result<ValueType> {
        let max_length = if self.eat(TokenKind::LParen).is_some() {
            let max_length = self.parse_positive_u64_literal("character string max length")?;
            self.expect(TokenKind::RParen)?;
            Some(max_length)
        } else {
            None
        };
        Ok(ValueType::CharacterString { kind, max_length })
    }

    fn peek_boolean_type_kind(&self) -> Option<BooleanTypeKind> {
        let text = self.peek().text.to_ascii_uppercase();
        match text.as_str() {
            "BOOL" => Some(BooleanTypeKind::Bool),
            "BOOLEAN" => Some(BooleanTypeKind::Boolean),
            _ => None,
        }
    }

    fn peek_temporal_type_kind(&self) -> Option<(TemporalTypeKind, usize)> {
        if self.peek_type_words(&["ZONED", "DATETIME"]) {
            return Some((TemporalTypeKind::ZonedDateTime, 2));
        }
        if self.peek_type_words(&["TIMESTAMP", "WITH", "TIME", "ZONE"]) {
            return Some((TemporalTypeKind::ZonedDateTime, 4));
        }
        if self.peek_type_words(&["LOCAL", "DATETIME"]) {
            return Some((TemporalTypeKind::LocalDateTime, 2));
        }
        if self.peek_type_words(&["TIMESTAMP", "WITHOUT", "TIME", "ZONE"]) {
            return Some((TemporalTypeKind::LocalDateTime, 4));
        }
        if self.peek_type_words(&["TIMESTAMP"]) {
            return Some((TemporalTypeKind::LocalDateTime, 1));
        }
        if self.at(TokenKind::Date) {
            return Some((TemporalTypeKind::Date, 1));
        }
        if self.peek_type_words(&["ZONED", "TIME"]) {
            return Some((TemporalTypeKind::ZonedTime, 2));
        }
        if self.peek_type_words(&["TIME", "WITH", "TIME", "ZONE"]) {
            return Some((TemporalTypeKind::ZonedTime, 4));
        }
        if self.peek_type_words(&["LOCAL", "TIME"]) {
            return Some((TemporalTypeKind::LocalTime, 2));
        }
        if self.peek_type_words(&["TIME", "WITHOUT", "TIME", "ZONE"]) {
            return Some((TemporalTypeKind::LocalTime, 4));
        }
        if self.at(TokenKind::Duration) {
            return Some((TemporalTypeKind::Duration, 1));
        }
        None
    }

    fn peek_byte_string_type_kind(&self) -> Option<ByteStringTypeKind> {
        let text = self.peek().text.to_ascii_uppercase();
        match text.as_str() {
            "BYTES" => Some(ByteStringTypeKind::Bytes),
            "BINARY" => Some(ByteStringTypeKind::Binary),
            "VARBINARY" => Some(ByteStringTypeKind::Varbinary),
            _ => None,
        }
    }

    fn parse_byte_string_type(&mut self, kind: ByteStringTypeKind) -> Result<ValueType> {
        let (min_length, max_length) = if self.eat(TokenKind::LParen).is_some() {
            match kind {
                ByteStringTypeKind::Bytes => {
                    let first = self.parse_u64_literal()?;
                    let (min_length, max_length) = if self.eat(TokenKind::Comma).is_some() {
                        (
                            Some(first),
                            Some(self.parse_positive_u64_literal("byte string max length")?),
                        )
                    } else {
                        self.ensure_positive_u64(first, "byte string max length")?;
                        (Some(0), Some(first))
                    };
                    self.expect(TokenKind::RParen)?;
                    if let (Some(min_length), Some(max_length)) = (min_length, max_length) {
                        if min_length > max_length {
                            return Err(Error::Message {
                                offset: self.peek().offset,
                                message: "byte string min length exceeds max length".to_owned(),
                            });
                        }
                    }
                    (min_length, max_length)
                }
                ByteStringTypeKind::Binary | ByteStringTypeKind::Varbinary => {
                    let length = match kind {
                        ByteStringTypeKind::Binary => {
                            self.parse_positive_u64_literal("byte string fixed length")?
                        }
                        ByteStringTypeKind::Varbinary => {
                            self.parse_positive_u64_literal("byte string max length")?
                        }
                        ByteStringTypeKind::Bytes => unreachable!(),
                    };
                    self.expect(TokenKind::RParen)?;
                    match kind {
                        ByteStringTypeKind::Binary => (Some(length), Some(length)),
                        ByteStringTypeKind::Varbinary => (Some(0), Some(length)),
                        ByteStringTypeKind::Bytes => unreachable!(),
                    }
                }
            }
        } else {
            match kind {
                ByteStringTypeKind::Binary => (Some(1), Some(1)),
                ByteStringTypeKind::Bytes | ByteStringTypeKind::Varbinary => (Some(0), None),
            }
        };
        Ok(ValueType::ByteString {
            kind,
            min_length,
            max_length,
        })
    }

    fn peek_exact_numeric_type_kind(&self) -> Option<ExactNumericTypeKind> {
        let text = self.peek().text.to_ascii_uppercase();
        match text.as_str() {
            "DEC" | "DECIMAL" => Some(ExactNumericTypeKind::Decimal),
            _ => None,
        }
    }

    fn parse_exact_numeric_type(&mut self, kind: ExactNumericTypeKind) -> Result<ValueType> {
        let (precision, scale) = self.parse_optional_precision_scale()?;
        if let (Some(precision), Some(scale)) = (precision, scale) {
            if scale > precision {
                return Err(Error::Message {
                    offset: self.peek().offset,
                    message: "numeric scale exceeds precision".to_owned(),
                });
            }
        }
        Ok(ValueType::ExactNumeric {
            kind,
            signed: None,
            precision,
            scale,
        })
    }

    fn parse_signed_or_unsigned_integer_type(&mut self) -> Result<ValueType> {
        let signed = self.bump().text.eq_ignore_ascii_case("SIGNED");
        self.parse_integer_numeric_type(Some(signed))
    }

    fn peek_integer_numeric_type_start(&self) -> bool {
        let first = self.peek().text.to_ascii_uppercase();
        Self::numeric_suffix(&first, "INTEGER").is_some()
            || Self::numeric_suffix(&first, "INT").is_some()
            || Self::numeric_suffix(&first, "UINT").is_some()
            || first == "BIGINT"
            || first == "SMALLINT"
            || first == "UBIGINT"
            || first == "USMALLINT"
            || (first == "SMALL" && self.peek_type_words(&["INTEGER"]))
            || (first == "BIG" && self.peek_type_words(&["INTEGER"]))
    }

    fn numeric_suffix(text: &str, prefix: &str) -> Option<Option<u64>> {
        let suffix = text.strip_prefix(prefix)?;
        if suffix.is_empty() {
            return Some(None);
        }
        if suffix.chars().all(|ch| ch.is_ascii_digit()) {
            return suffix.parse().ok().map(Some);
        }
        None
    }

    fn parse_integer_numeric_type(&mut self, explicit_signed: Option<bool>) -> Result<ValueType> {
        let first = self.parse_identifier()?.value.to_ascii_uppercase();
        let mut signed = explicit_signed;
        let mut precision = None;

        match first.as_str() {
            "SMALL" | "BIG" => {
                self.expect_identifier_word("INTEGER")?;
                signed.get_or_insert(true);
            }
            "INTEGER" | "INT" | "UINT" => {
                if first == "UINT" {
                    signed = Some(false);
                } else {
                    signed.get_or_insert(true);
                }
                precision = self.parse_optional_single_precision()?;
            }
            "SMALLINT" | "BIGINT" => {
                signed.get_or_insert(true);
            }
            "USMALLINT" | "UBIGINT" => {
                signed = Some(false);
            }
            text if Self::numeric_suffix(text, "UINT").is_some() => {
                signed = Some(false);
                precision = Self::numeric_suffix(text, "UINT").flatten();
                if let Some(precision) = precision {
                    self.ensure_standard_integer_suffix(precision)?;
                }
            }
            text if Self::numeric_suffix(text, "INT").is_some() => {
                signed.get_or_insert(true);
                precision = Self::numeric_suffix(text, "INT").flatten();
                if let Some(precision) = precision {
                    self.ensure_standard_integer_suffix(precision)?;
                }
            }
            text if Self::numeric_suffix(text, "INTEGER").is_some() => {
                signed.get_or_insert(true);
                precision = Self::numeric_suffix(text, "INTEGER").flatten();
                if let Some(precision) = precision {
                    self.ensure_standard_integer_suffix(precision)?;
                }
            }
            _ => {
                return Err(self.error_expected(vec![TokenKind::Identifier], self.peek_kind()));
            }
        }

        if let Some(precision) = precision {
            self.ensure_positive_u64(precision, "numeric precision")?;
        }

        Ok(ValueType::ExactNumeric {
            kind: ExactNumericTypeKind::Integer,
            signed,
            precision,
            scale: None,
        })
    }

    fn ensure_standard_integer_suffix(&self, precision: u64) -> Result<()> {
        if !matches!(precision, 8 | 16 | 32 | 64 | 128 | 256) {
            return Err(Error::Message {
                offset: self.peek().offset,
                message: "integer type suffix must be one of 8, 16, 32, 64, 128, or 256".to_owned(),
            });
        }
        Ok(())
    }

    fn parse_optional_single_precision(&mut self) -> Result<Option<u64>> {
        if self.eat(TokenKind::LParen).is_none() {
            return Ok(None);
        }
        let precision = self.parse_positive_u64_literal("numeric precision")?;
        self.expect(TokenKind::RParen)?;
        Ok(Some(precision))
    }

    fn peek_approximate_numeric_type_kind(&self) -> Option<ApproximateNumericTypeKind> {
        let text = self.peek().text.to_ascii_uppercase();
        if Self::numeric_suffix(&text, "FLOAT").is_some() {
            return Some(ApproximateNumericTypeKind::Float);
        }
        match text.as_str() {
            "REAL" => Some(ApproximateNumericTypeKind::Real),
            "DOUBLE" => Some(ApproximateNumericTypeKind::Double),
            _ => None,
        }
    }

    fn parse_approximate_numeric_type(
        &mut self,
        kind: ApproximateNumericTypeKind,
        type_name: String,
    ) -> Result<ValueType> {
        let mut precision = None;
        let mut scale = None;
        match kind {
            ApproximateNumericTypeKind::Float => {
                let text = type_name.to_ascii_uppercase();
                precision = Self::numeric_suffix(&text, "FLOAT").flatten();
                if let Some(precision) = precision {
                    self.ensure_standard_float_suffix(precision)?;
                    self.ensure_approximate_numeric_precision(precision)?;
                }
                if precision.is_none() {
                    (precision, scale) = self.parse_optional_precision_scale()?;
                    if let Some(precision) = precision {
                        self.ensure_approximate_numeric_precision(precision)?;
                    }
                }
            }
            ApproximateNumericTypeKind::Real => {}
            ApproximateNumericTypeKind::Double => {
                if self.peek_type_words(&["PRECISION"]) {
                    self.bump();
                }
            }
        }
        Ok(ValueType::ApproximateNumeric {
            kind,
            precision,
            scale,
        })
    }

    fn parse_optional_precision_scale(&mut self) -> Result<(Option<u64>, Option<u64>)> {
        if self.eat(TokenKind::LParen).is_none() {
            return Ok((None, None));
        }
        let precision = self.parse_positive_u64_literal("numeric precision")?;
        let scale = if self.eat(TokenKind::Comma).is_some() {
            Some(self.parse_u64_literal()?)
        } else {
            None
        };
        self.expect(TokenKind::RParen)?;
        Ok((Some(precision), scale))
    }

    fn ensure_approximate_numeric_precision(&self, precision: u64) -> Result<()> {
        if precision < 2 {
            return Err(Error::Message {
                offset: self.peek().offset,
                message: "approximate numeric precision must be greater than or equal to 2"
                    .to_owned(),
            });
        }
        Ok(())
    }

    fn ensure_standard_float_suffix(&self, precision: u64) -> Result<()> {
        if !matches!(precision, 16 | 32 | 64 | 128 | 256) {
            return Err(Error::Message {
                offset: self.peek().offset,
                message: "FLOAT type suffix must be one of 16, 32, 64, 128, or 256".to_owned(),
            });
        }
        Ok(())
    }

    fn parse_optional_node_type_definition(&mut self) -> Result<Option<NodeTypeDefinition>> {
        if !self.eat_node_synonym() {
            return Err(
                self.error_expected(vec![TokenKind::Node, TokenKind::Vertex], self.peek_kind())
            );
        }
        if self.at(TokenKind::Type) && self.peek_n_kind(1) == Some(TokenKind::Identifier) {
            self.bump();
        }
        if !matches!(self.peek_kind(), TokenKind::Identifier) && !self.starts_node_type_filler() {
            return Ok(None);
        }
        let name = if self.starts_node_type_filler() {
            Identifier::new("")
        } else {
            self.parse_identifier()?
        };
        let labels = self.parse_optional_type_label_list()?;
        let properties = self.parse_optional_property_type_set()?;
        Ok(Some(NodeTypeDefinition {
            name,
            labels,
            properties,
        }))
    }

    fn parse_optional_edge_type_definition(&mut self) -> Result<Option<EdgeTypeDefinition>> {
        self.parse_optional_edge_type_definition_with_direction(Direction::Right, None)
    }

    fn parse_optional_edge_type_definition_with_direction(
        &mut self,
        default_direction: Direction,
        explicit_edge_kind: Option<Direction>,
    ) -> Result<Option<EdgeTypeDefinition>> {
        if !self.eat_edge_synonym() {
            return Err(self.error_expected(
                vec![TokenKind::Edge, TokenKind::Relationship],
                self.peek_kind(),
            ));
        }
        if self.at(TokenKind::Type) && self.peek_n_kind(1) == Some(TokenKind::Identifier) {
            self.bump();
        }
        if !matches!(self.peek_kind(), TokenKind::Identifier) && !self.starts_node_type_filler() {
            return Ok(None);
        }
        let name = if self.starts_node_type_filler() {
            Identifier::new("")
        } else {
            self.parse_identifier()?
        };
        let labels = self.parse_optional_type_label_list()?;
        let (direction, source, destination, properties) =
            self.parse_edge_type_tail(default_direction, explicit_edge_kind)?;
        Ok(Some(EdgeTypeDefinition {
            name,
            direction,
            labels,
            source,
            destination,
            properties,
        }))
    }

    fn parse_field_type_specification(&mut self) -> Result<Vec<FieldType>> {
        self.expect(TokenKind::LBrace)?;
        let mut fields = Vec::new();
        if !self.at(TokenKind::RBrace) {
            loop {
                let offset = self.peek().offset;
                let name = self.parse_identifier()?;
                if fields
                    .iter()
                    .any(|field: &FieldType| field.name.value.eq_ignore_ascii_case(&name.value))
                {
                    return Err(Error::Message {
                        offset,
                        message: format!("duplicate field '{}'", name.value),
                    });
                }
                self.parse_optional_typed_marker()?;
                let value_type = self.parse_value_type()?;
                fields.push(FieldType { name, value_type });
                if self.eat(TokenKind::Comma).is_none() {
                    break;
                }
            }
        }
        self.expect(TokenKind::RBrace)?;
        Ok(fields)
    }

    fn parse_component_type_list(&mut self) -> Result<Vec<ValueType>> {
        let mut components = vec![self.parse_value_type_component()?];
        while self.eat(TokenKind::Pipe).is_some() {
            components.push(self.parse_value_type_component()?);
        }
        validate_dynamic_union_component_nullability(&components, self.peek().offset)?;
        Ok(components)
    }

    fn parse_optional_list_max_length(&mut self) -> Result<Option<u64>> {
        if self.eat(TokenKind::LBracket).is_none() {
            return Ok(None);
        }
        let max_length = self.parse_u64_literal()?;
        self.ensure_positive_u64(max_length, "list max length")?;
        self.expect(TokenKind::RBracket)?;
        Ok(Some(max_length))
    }

    fn peek_list_value_type_name(&self) -> Option<(TokenKind, bool)> {
        match self.peek_kind() {
            TokenKind::List | TokenKind::Array => Some((self.peek_kind(), false)),
            _ => None,
        }
    }

    fn parse_list_value_type_name(&mut self) -> Result<(TokenKind, bool)> {
        if self.at(TokenKind::List) || self.at(TokenKind::Array) {
            let token = self.bump();
            return Ok((token.kind, false));
        }
        Err(self.error_expected(vec![TokenKind::List, TokenKind::Array], self.peek_kind()))
    }

    fn wrap_list_value_type(
        element: ValueType,
        kind: TokenKind,
        group: bool,
        max_length: Option<u64>,
        offset: usize,
    ) -> Result<ValueType> {
        if group {
            return Err(Error::Message {
                offset,
                message: "GROUP list value type names are not user-visible standard GQL syntax"
                    .to_owned(),
            });
        }
        let value_type = match (kind, group, max_length) {
            (TokenKind::List, false, Some(max_length)) => ValueType::ListWithLength {
                element: Box::new(element),
                max_length,
            },
            (TokenKind::List, false, None) => ValueType::List(Box::new(element)),
            (TokenKind::Array, false, Some(max_length)) => ValueType::ArrayWithLength {
                element: Box::new(element),
                max_length,
            },
            (TokenKind::Array, false, None) => ValueType::Array(Box::new(element)),
            _ => unreachable!("only LIST and ARRAY token kinds wrap list value types"),
        };
        Ok(value_type)
    }

    fn parse_value_type_name(&mut self) -> Result<Identifier> {
        let mut parts = vec![self.parse_identifier()?.value];
        match parts[0].to_ascii_uppercase().as_str() {
            "BIG" | "SMALL" if self.peek_type_words(&["INTEGER"]) => {
                parts.push(self.bump().text);
            }
            "DOUBLE" if self.peek_type_words(&["PRECISION"]) => {
                parts.push(self.bump().text);
            }
            "CHAR" | "CHARACTER" if self.peek_type_words(&["VARYING"]) => {
                parts.push(self.bump().text);
            }
            "CHAR" | "CHARACTER" if self.peek_type_words(&["LARGE", "OBJECT"]) => {
                parts.push(self.bump().text);
                parts.push(self.bump().text);
            }
            "TIME" | "TIMESTAMP" | "DATETIME"
                if self.peek_type_words(&["WITH", "TIME", "ZONE"])
                    || self.peek_type_words(&["WITHOUT", "TIME", "ZONE"]) =>
            {
                parts.push(self.bump().text);
                parts.push(self.bump().text);
                parts.push(self.bump().text);
            }
            "LOCAL" if self.peek_type_words(&["DATETIME"]) || self.peek_type_words(&["TIME"]) => {
                parts.push(self.bump().text);
            }
            "ZONED" if self.peek_type_words(&["DATETIME"]) || self.peek_type_words(&["TIME"]) => {
                parts.push(self.bump().text);
            }
            _ => {}
        }
        Ok(Identifier::new(parts.join(" ")))
    }

    fn peek_type_words(&self, words: &[&str]) -> bool {
        words.iter().enumerate().all(|(offset, word)| {
            self.tokens.get(self.pos + offset).is_some_and(|token| {
                is_identifier_like(token.kind) && token.text.eq_ignore_ascii_case(word)
            })
        })
    }

    fn expect_identifier_word(&mut self, word: &str) -> Result<Token> {
        if is_identifier_like(self.peek_kind()) && self.peek().text.eq_ignore_ascii_case(word) {
            Ok(self.bump())
        } else if self.at(TokenKind::Eof) {
            Err(self.eof_error())
        } else {
            Err(self.error_expected(vec![TokenKind::Identifier], self.peek_kind()))
        }
    }

    fn parse_pattern_list(&mut self) -> Result<Vec<PathPattern>> {
        let mut patterns = Vec::new();
        loop {
            patterns.push(self.parse_path_pattern()?);
            if self.eat(TokenKind::Comma).is_none() {
                break;
            }
        }
        Ok(patterns)
    }

    fn parse_insert_pattern_list(&mut self) -> Result<Vec<PathPattern>> {
        let mut patterns = Vec::new();
        loop {
            patterns.push(self.parse_insert_path_pattern()?);
            if self.eat(TokenKind::Comma).is_none() {
                break;
            }
        }
        Ok(patterns)
    }

    fn parse_insert_path_pattern(&mut self) -> Result<PathPattern> {
        let start = self.parse_insert_node_pattern()?;
        let mut chains = Vec::new();
        while self.starts_insert_relationship() {
            let relationship = self.parse_insert_relationship_pattern()?;
            let node = self.parse_insert_node_pattern()?;
            chains.push(PathPatternChain { relationship, node });
        }
        let factors = path_pattern_factors_from_flat_path(&start, &chains);
        Ok(PathPattern {
            variable: None,
            prefix: None,
            parenthesized: None,
            start,
            factors,
            alternation: None,
            chains,
            alternatives: Vec::new(),
        })
    }

    fn parse_path_pattern(&mut self) -> Result<PathPattern> {
        let variable =
            if is_identifier_like(self.peek_kind()) && self.peek_n_kind(1) == Some(TokenKind::Eq) {
                let variable = self.parse_identifier()?;
                self.expect(TokenKind::Eq)?;
                Some(variable)
            } else {
                None
            };
        let prefix = self.parse_optional_path_pattern_prefix()?;
        if self.starts_parenthesized_path_pattern_expression() {
            let parenthesized = self.parse_parenthesized_path_pattern_expression()?;
            let start = parenthesized.pattern.start.clone();
            let chains = parenthesized.pattern.chains.clone();
            let factors = vec![PathPatternFactor::Parenthesized(Box::new(
                parenthesized.clone(),
            ))];
            let alternation = parenthesized.pattern.alternation;
            let alternatives = parenthesized.pattern.alternatives.clone();
            return Ok(PathPattern {
                variable,
                prefix,
                parenthesized: Some(parenthesized),
                start,
                chains,
                factors,
                alternation,
                alternatives,
            });
        }

        let PathPatternTerm {
            start,
            chains,
            factors,
        } = self.parse_path_pattern_term()?;
        let alternation = self.peek_path_pattern_alternation_operator();
        let mut alternatives = Vec::new();
        if let Some(alternation) = alternation {
            self.parse_path_pattern_alternation_operator()?;
            alternatives.push(self.parse_path_pattern_term()?);
            while self.peek_path_pattern_alternation_operator() == Some(alternation) {
                self.parse_path_pattern_alternation_operator()?;
                alternatives.push(self.parse_path_pattern_term()?);
            }
        }
        Ok(PathPattern {
            variable,
            prefix,
            parenthesized: None,
            start,
            chains,
            factors,
            alternation,
            alternatives,
        })
    }

    fn starts_parenthesized_path_pattern_expression(&self) -> bool {
        if self.peek_kind() != TokenKind::LParen {
            return false;
        }

        matches!(
            self.peek_n_kind(1),
            Some(
                TokenKind::LParen
                    | TokenKind::Walk
                    | TokenKind::Trail
                    | TokenKind::Simple
                    | TokenKind::Acyclic
                    | TokenKind::Minus
                    | TokenKind::ArrowLeft
                    | TokenKind::ArrowRight
                    | TokenKind::Tilde
            )
        ) || (self.peek_n_kind(1) == Some(TokenKind::Lt)
            && self.peek_n_kind(2) == Some(TokenKind::Tilde))
            || self.peek_n_kind(1).is_some_and(is_identifier_like)
                && self.peek_n_kind(2) == Some(TokenKind::Eq)
    }

    fn parse_parenthesized_path_pattern_expression(
        &mut self,
    ) -> Result<ParenthesizedPathPatternExpression> {
        self.expect(TokenKind::LParen)?;
        let variable =
            if is_identifier_like(self.peek_kind()) && self.peek_n_kind(1) == Some(TokenKind::Eq) {
                let variable = self.parse_identifier()?;
                self.expect(TokenKind::Eq)?;
                Some(variable)
            } else {
                None
            };
        let prefix = self.parse_optional_path_mode_prefix();
        let pattern = self.parse_path_pattern()?;
        let where_clause = if self.eat(TokenKind::Where).is_some() {
            Some(self.parse_expr()?)
        } else {
            None
        };
        if let (Some(variable), Some(where_clause)) = (&variable, &where_clause) {
            if expr_references_identifier(where_clause, variable) {
                return Err(Error::Message {
                    offset: self.peek().offset,
                    message: format!(
                        "parenthesized path pattern WHERE cannot reference path variable '{}'",
                        variable.value
                    ),
                });
            }
        }
        self.expect(TokenKind::RParen)?;

        let questioned = self.eat(TokenKind::Question).is_some();
        let quantifier = if questioned {
            None
        } else {
            self.parse_optional_path_quantifier()?
        };

        Ok(ParenthesizedPathPatternExpression {
            variable,
            prefix,
            pattern: Box::new(pattern),
            where_clause,
            quantifier,
            questioned,
        })
    }

    fn parse_path_pattern_term(&mut self) -> Result<PathPatternTerm> {
        let mut factors = self.parse_path_pattern_factor()?;
        while self.starts_path_pattern_factor() {
            factors.extend(self.parse_path_pattern_factor()?);
        }

        self.path_pattern_term_from_factors(factors)
    }

    fn starts_path_pattern_factor(&self) -> bool {
        self.starts_parenthesized_path_pattern_expression()
            || self.starts_simplified_path_pattern_expression()
            || self.at(TokenKind::LParen)
            || self.starts_relationship()
    }

    fn parse_path_pattern_factor(&mut self) -> Result<Vec<PathPatternFactor>> {
        if self.starts_parenthesized_path_pattern_expression() {
            return Ok(vec![PathPatternFactor::Parenthesized(Box::new(
                self.parse_parenthesized_path_pattern_expression()?,
            ))]);
        }
        if self.starts_simplified_path_pattern_expression() {
            return Ok(match self.parse_simplified_path_pattern_expression()? {
                SimplifiedPathPatternExpression::Linear(relationships) => relationships
                    .into_iter()
                    .map(PathPatternFactor::Relationship)
                    .collect(),
                SimplifiedPathPatternExpression::Alternation {
                    alternation,
                    alternatives,
                } => vec![PathPatternFactor::Alternation {
                    alternation,
                    alternatives: alternatives
                        .into_iter()
                        .map(|relationships| {
                            relationships
                                .into_iter()
                                .map(PathPatternFactor::Relationship)
                                .collect()
                        })
                        .collect(),
                }],
            });
        }
        if self.at(TokenKind::LParen) {
            return Ok(vec![PathPatternFactor::Node(self.parse_node_pattern()?)]);
        }
        if self.starts_relationship() {
            return Ok(vec![PathPatternFactor::Relationship(
                self.parse_relationship_pattern()?,
            )]);
        }

        Err(self.error_expected(vec![TokenKind::LParen], self.peek_kind()))
    }

    fn starts_simplified_path_pattern_expression(&self) -> bool {
        matches!(
            (self.peek_kind(), self.peek_n_kind(1), self.peek_n_kind(2)),
            (TokenKind::Minus, Some(TokenKind::Slash), _)
                | (TokenKind::Tilde, Some(TokenKind::Slash), _)
                | (TokenKind::ArrowLeft, Some(TokenKind::Slash), _)
                | (
                    TokenKind::Lt,
                    Some(TokenKind::Tilde),
                    Some(TokenKind::Slash)
                )
        )
    }

    fn parse_simplified_path_pattern_expression(
        &mut self,
    ) -> Result<SimplifiedPathPatternExpression> {
        let prefix = self.parse_simplified_delimiter_prefix()?;
        let mut branches = vec![self.parse_simplified_relationship_body_sequence()?];
        let alternation = self.peek_path_pattern_alternation_operator();
        if let Some(alternation) = alternation {
            self.parse_path_pattern_alternation_operator()?;
            branches.push(self.parse_simplified_relationship_body_sequence()?);
            while self.peek_path_pattern_alternation_operator() == Some(alternation) {
                self.parse_path_pattern_alternation_operator()?;
                branches.push(self.parse_simplified_relationship_body_sequence()?);
            }
        }
        let direction = self.parse_simplified_delimiter_suffix(prefix)?;
        let quantifier = self.parse_optional_path_quantifier()?;

        let alternatives = branches
            .into_iter()
            .map(|bodies| self.simplified_relationships_from_bodies(bodies, direction, &quantifier))
            .collect::<Vec<_>>();

        if let Some(alternation) = alternation {
            Ok(SimplifiedPathPatternExpression::Alternation {
                alternation,
                alternatives,
            })
        } else {
            Ok(SimplifiedPathPatternExpression::Linear(
                alternatives.into_iter().next().unwrap_or_default(),
            ))
        }
    }

    fn parse_simplified_relationship_body_sequence(
        &mut self,
    ) -> Result<Vec<SimplifiedRelationshipBody>> {
        let mut bodies = vec![self.parse_simplified_relationship_body()?];
        while !matches!(self.peek_kind(), TokenKind::Slash | TokenKind::Pipe) {
            bodies.push(self.parse_simplified_relationship_body()?);
        }
        Ok(bodies)
    }

    fn simplified_relationships_from_bodies(
        &self,
        bodies: Vec<SimplifiedRelationshipBody>,
        direction: Direction,
        quantifier: &Option<PathPatternQuantifier>,
    ) -> Vec<RelationshipPattern> {
        let last = bodies.len().saturating_sub(1);
        bodies
            .into_iter()
            .enumerate()
            .map(|(index, body)| RelationshipPattern {
                direction: body.direction_override.unwrap_or(direction),
                variable: None,
                temporary: false,
                labels: body.labels,
                label_expression: Some(body.label_expression),
                properties: None,
                where_clause: None,
                quantifier: body.quantifier.or_else(|| {
                    if index == last {
                        quantifier.clone()
                    } else {
                        None
                    }
                }),
            })
            .collect()
    }

    fn parse_simplified_relationship_body(&mut self) -> Result<SimplifiedRelationshipBody> {
        let leading_override = self.parse_optional_simplified_direction_override_prefix()?;
        let label_expression = self.parse_simplified_label_expression()?;
        let mut labels = Vec::new();
        collect_label_names(&label_expression, &mut labels);
        Ok(SimplifiedRelationshipBody {
            labels,
            label_expression,
            direction_override: self
                .parse_optional_simplified_direction_override_suffix(leading_override)?,
            quantifier: self.parse_optional_path_quantifier()?,
        })
    }

    fn parse_simplified_label_expression(&mut self) -> Result<LabelExpression> {
        self.parse_label_and()
    }

    fn parse_optional_simplified_direction_override_prefix(&mut self) -> Result<Option<Direction>> {
        if self.eat(TokenKind::Lt).is_some() {
            if self.eat(TokenKind::Tilde).is_some() {
                return Ok(Some(Direction::LeftOrUndirected));
            }
            return Ok(Some(Direction::Left));
        }
        if self.eat(TokenKind::Tilde).is_some() {
            return Ok(Some(Direction::Undirected));
        }
        if self.eat(TokenKind::Minus).is_some() {
            return Ok(Some(Direction::Any));
        }
        Ok(None)
    }

    fn parse_optional_simplified_direction_override_suffix(
        &mut self,
        leading_override: Option<Direction>,
    ) -> Result<Option<Direction>> {
        if self.eat(TokenKind::Gt).is_some() {
            return Ok(Some(match leading_override {
                Some(Direction::Left) => Direction::LeftOrRight,
                Some(Direction::Undirected) => Direction::UndirectedOrRight,
                Some(direction) => direction,
                None => Direction::Right,
            }));
        }
        Ok(leading_override)
    }

    fn parse_simplified_delimiter_prefix(&mut self) -> Result<SimplifiedPrefix> {
        if self.eat(TokenKind::Minus).is_some() {
            self.expect(TokenKind::Slash)?;
            return Ok(SimplifiedPrefix::Minus);
        }
        if self.eat(TokenKind::Tilde).is_some() {
            self.expect(TokenKind::Slash)?;
            return Ok(SimplifiedPrefix::Tilde);
        }
        if self.eat(TokenKind::ArrowLeft).is_some() {
            self.expect(TokenKind::Slash)?;
            return Ok(SimplifiedPrefix::ArrowLeft);
        }
        self.expect(TokenKind::Lt)?;
        self.expect(TokenKind::Tilde)?;
        self.expect(TokenKind::Slash)?;
        Ok(SimplifiedPrefix::LtTilde)
    }

    fn parse_simplified_delimiter_suffix(&mut self, prefix: SimplifiedPrefix) -> Result<Direction> {
        self.expect(TokenKind::Slash)?;
        match prefix {
            SimplifiedPrefix::Minus => {
                if self.eat(TokenKind::ArrowRight).is_some() {
                    Ok(Direction::Right)
                } else {
                    self.expect(TokenKind::Minus)?;
                    Ok(Direction::Any)
                }
            }
            SimplifiedPrefix::ArrowLeft => {
                if self.eat(TokenKind::ArrowRight).is_some() {
                    Ok(Direction::LeftOrRight)
                } else {
                    self.expect(TokenKind::Minus)?;
                    Ok(Direction::Left)
                }
            }
            SimplifiedPrefix::Tilde => {
                self.expect(TokenKind::Tilde)?;
                if self.eat(TokenKind::Gt).is_some() {
                    Ok(Direction::UndirectedOrRight)
                } else {
                    Ok(Direction::Undirected)
                }
            }
            SimplifiedPrefix::LtTilde => {
                self.expect(TokenKind::Tilde)?;
                Ok(Direction::LeftOrUndirected)
            }
        }
    }

    fn path_pattern_term_from_factors(
        &self,
        factors: Vec<PathPatternFactor>,
    ) -> Result<PathPatternTerm> {
        let mut start = None;
        let mut chains = Vec::new();
        let mut pending_relationship = None;
        for factor in &factors {
            apply_path_pattern_factor_to_term(
                factor,
                &mut start,
                &mut chains,
                &mut pending_relationship,
            );
        }
        if let Some(relationship) = pending_relationship {
            if start.is_none() {
                start = Some(empty_node_pattern());
            }
            chains.push(PathPatternChain {
                relationship,
                node: empty_node_pattern(),
            });
        }

        let start =
            start.ok_or_else(|| self.error_expected(vec![TokenKind::LParen], self.peek_kind()))?;

        Ok(PathPatternTerm {
            start,
            chains,
            factors,
        })
    }

    fn peek_path_pattern_alternation_operator(&self) -> Option<PathPatternAlternation> {
        if self.peek_kind() == TokenKind::Pipe && self.peek_n_kind(1) == Some(TokenKind::Plus) {
            Some(PathPatternAlternation::Multiset)
        } else if self.peek_kind() == TokenKind::Pipe {
            Some(PathPatternAlternation::Union)
        } else {
            None
        }
    }

    fn parse_path_pattern_alternation_operator(&mut self) -> Result<PathPatternAlternation> {
        self.expect(TokenKind::Pipe)?;
        if self.eat(TokenKind::Plus).is_some() {
            self.expect(TokenKind::Pipe)?;
            Ok(PathPatternAlternation::Multiset)
        } else {
            Ok(PathPatternAlternation::Union)
        }
    }

    fn parse_optional_path_pattern_prefix(&mut self) -> Result<Option<PathPatternPrefix>> {
        if matches!(
            self.peek_kind(),
            TokenKind::All | TokenKind::Any | TokenKind::Shortest
        ) {
            return self.parse_optional_path_search_prefix();
        }

        Ok(self.parse_optional_path_mode_prefix())
    }

    fn parse_optional_path_mode_prefix(&mut self) -> Option<PathPatternPrefix> {
        let mode = self.parse_optional_path_mode()?;

        let path_or_paths = self.parse_optional_path_or_paths();

        Some(PathPatternPrefix::Mode {
            mode,
            path_or_paths,
        })
    }

    fn parse_required_path_pattern_prefix(&mut self) -> Result<PathPatternPrefix> {
        self.parse_optional_path_pattern_prefix()?.ok_or_else(|| {
            self.error_expected(
                vec![
                    TokenKind::Walk,
                    TokenKind::Trail,
                    TokenKind::Simple,
                    TokenKind::Acyclic,
                    TokenKind::All,
                    TokenKind::Any,
                    TokenKind::Shortest,
                ],
                self.peek_kind(),
            )
        })
    }

    fn parse_optional_path_search_prefix(&mut self) -> Result<Option<PathPatternPrefix>> {
        let search = if self.eat(TokenKind::All).is_some() {
            if self.eat(TokenKind::Shortest).is_some() {
                PathSearchPrefix::AllShortest {
                    mode: self.parse_optional_path_mode(),
                    path_or_paths: self.parse_optional_path_or_paths(),
                }
            } else {
                PathSearchPrefix::All {
                    mode: self.parse_optional_path_mode(),
                    path_or_paths: self.parse_optional_path_or_paths(),
                }
            }
        } else if self.eat(TokenKind::Any).is_some() {
            if self.eat(TokenKind::Shortest).is_some() {
                PathSearchPrefix::AnyShortest {
                    mode: self.parse_optional_path_mode(),
                    path_or_paths: self.parse_optional_path_or_paths(),
                }
            } else {
                let count = self.parse_optional_unsigned_integer_specification()?;
                PathSearchPrefix::Any {
                    count,
                    mode: self.parse_optional_path_mode(),
                    path_or_paths: self.parse_optional_path_or_paths(),
                }
            }
        } else if self.eat(TokenKind::Shortest).is_some() {
            let count = self.parse_optional_unsigned_integer_specification()?;
            let mode = self.parse_optional_path_mode();
            let path_or_paths = self.parse_optional_path_or_paths();

            if self.eat(TokenKind::Group).is_some() {
                PathSearchPrefix::CountedShortestGroup {
                    count,
                    mode,
                    path_or_paths,
                    groups: false,
                }
            } else if self.eat(TokenKind::Groups).is_some() {
                PathSearchPrefix::CountedShortestGroup {
                    count,
                    mode,
                    path_or_paths,
                    groups: true,
                }
            } else if let Some(count) = count {
                PathSearchPrefix::CountedShortest {
                    count,
                    mode,
                    path_or_paths,
                }
            } else {
                PathSearchPrefix::Shortest {
                    mode,
                    path_or_paths,
                }
            }
        } else {
            return Ok(None);
        };

        Ok(Some(PathPatternPrefix::Search(search)))
    }

    fn parse_optional_unsigned_integer_specification(
        &mut self,
    ) -> Result<Option<UnsignedIntegerSpecification>> {
        if self.at(TokenKind::Integer) {
            return Ok(Some(UnsignedIntegerSpecification::Literal(
                self.parse_u64_literal()?,
            )));
        }
        if self.at(TokenKind::Parameter) {
            let token = self.bump();
            return Ok(Some(UnsignedIntegerSpecification::Parameter(
                token.text.trim_start_matches('$').to_owned(),
            )));
        }
        Ok(None)
    }

    fn parse_unsigned_integer_specification(&mut self) -> Result<UnsignedIntegerSpecification> {
        self.parse_optional_unsigned_integer_specification()?
            .ok_or_else(|| {
                self.error_expected(
                    vec![TokenKind::Integer, TokenKind::Parameter],
                    self.peek_kind(),
                )
            })
    }

    fn parse_optional_path_mode(&mut self) -> Option<PathMode> {
        if self.eat(TokenKind::Walk).is_some() {
            Some(PathMode::Walk)
        } else if self.eat(TokenKind::Trail).is_some() {
            Some(PathMode::Trail)
        } else if self.eat(TokenKind::Simple).is_some() {
            Some(PathMode::Simple)
        } else if self.eat(TokenKind::Acyclic).is_some() {
            Some(PathMode::Acyclic)
        } else {
            None
        }
    }

    fn parse_optional_path_or_paths(&mut self) -> Option<PathOrPaths> {
        if self.eat(TokenKind::Path).is_some() {
            Some(PathOrPaths::Path)
        } else if self.eat(TokenKind::Paths).is_some() {
            Some(PathOrPaths::Paths)
        } else {
            None
        }
    }

    fn starts_relationship(&self) -> bool {
        matches!(
            self.peek_kind(),
            TokenKind::Minus | TokenKind::ArrowLeft | TokenKind::ArrowRight | TokenKind::Tilde
        ) || (self.peek_kind() == TokenKind::Lt && self.peek_n_kind(1) == Some(TokenKind::Tilde))
    }

    fn starts_insert_relationship(&self) -> bool {
        matches!(
            self.peek_kind(),
            TokenKind::Minus | TokenKind::ArrowLeft | TokenKind::Tilde
        )
    }

    fn parse_node_pattern(&mut self) -> Result<NodePattern> {
        self.expect(TokenKind::LParen)?;
        let (variable, temporary, labels, label_expression, properties, where_clause) =
            self.parse_pattern_element_body(TokenKind::RParen)?;
        self.expect(TokenKind::RParen)?;
        let quantifier = self.parse_optional_path_quantifier()?;
        Ok(NodePattern {
            variable,
            temporary,
            labels,
            label_expression,
            properties,
            where_clause,
            quantifier,
        })
    }

    fn parse_insert_node_pattern(&mut self) -> Result<NodePattern> {
        self.expect(TokenKind::LParen)?;
        let (variable, labels, label_expression, properties) = if self.at(TokenKind::RParen) {
            (None, Vec::new(), None, None)
        } else {
            self.parse_insert_element_pattern_body(TokenKind::RParen)?
        };
        self.expect(TokenKind::RParen)?;
        Ok(NodePattern {
            variable,
            temporary: false,
            labels,
            label_expression,
            properties,
            where_clause: None,
            quantifier: None,
        })
    }

    fn parse_relationship_pattern(&mut self) -> Result<RelationshipPattern> {
        if self.eat(TokenKind::ArrowLeft).is_some() {
            let (
                direction,
                variable,
                temporary,
                labels,
                label_expression,
                properties,
                where_clause,
            ) = if self.eat(TokenKind::LBracket).is_some() {
                let (variable, temporary, labels, label_expression, properties, where_clause) =
                    self.parse_pattern_element_body(TokenKind::RBracket)?;
                self.expect(TokenKind::RBracket)?;
                let direction = if self.eat(TokenKind::ArrowRight).is_some() {
                    Direction::LeftOrRight
                } else {
                    self.expect(TokenKind::Minus)?;
                    Direction::Left
                };
                (
                    direction,
                    variable,
                    temporary,
                    labels,
                    label_expression,
                    properties,
                    where_clause,
                )
            } else {
                let direction = if self.eat(TokenKind::Gt).is_some() {
                    Direction::LeftOrRight
                } else {
                    Direction::Left
                };
                (direction, None, false, Vec::new(), None, None, None)
            };
            return Ok(RelationshipPattern {
                direction,
                variable,
                temporary,
                labels,
                label_expression,
                properties,
                where_clause,
                quantifier: self.parse_optional_path_quantifier()?,
            });
        }

        if self.eat(TokenKind::Lt).is_some() {
            self.expect(TokenKind::Tilde)?;
            if self.eat(TokenKind::LBracket).is_some() {
                let (variable, temporary, labels, label_expression, properties, where_clause) =
                    self.parse_pattern_element_body(TokenKind::RBracket)?;
                self.expect(TokenKind::RBracket)?;
                self.expect(TokenKind::Tilde)?;
                return Ok(RelationshipPattern {
                    direction: Direction::LeftOrUndirected,
                    variable,
                    temporary,
                    labels,
                    label_expression,
                    properties,
                    where_clause,
                    quantifier: self.parse_optional_path_quantifier()?,
                });
            }
            return Ok(RelationshipPattern {
                direction: Direction::LeftOrUndirected,
                variable: None,
                temporary: false,
                labels: Vec::new(),
                label_expression: None,
                properties: None,
                where_clause: None,
                quantifier: self.parse_optional_path_quantifier()?,
            });
        }

        if self.eat(TokenKind::ArrowRight).is_some() {
            return Ok(RelationshipPattern {
                direction: Direction::Right,
                variable: None,
                temporary: false,
                labels: Vec::new(),
                label_expression: None,
                properties: None,
                where_clause: None,
                quantifier: self.parse_optional_path_quantifier()?,
            });
        }

        if self.eat(TokenKind::Tilde).is_some() {
            let (
                direction,
                variable,
                temporary,
                labels,
                label_expression,
                properties,
                where_clause,
            ) = if self.eat(TokenKind::LBracket).is_some() {
                let (variable, temporary, labels, label_expression, properties, where_clause) =
                    self.parse_pattern_element_body(TokenKind::RBracket)?;
                self.expect(TokenKind::RBracket)?;
                self.expect(TokenKind::Tilde)?;
                let direction = if self.eat(TokenKind::Gt).is_some() {
                    Direction::UndirectedOrRight
                } else {
                    Direction::Undirected
                };
                (
                    direction,
                    variable,
                    temporary,
                    labels,
                    label_expression,
                    properties,
                    where_clause,
                )
            } else {
                let direction = if self.eat(TokenKind::Gt).is_some() {
                    Direction::UndirectedOrRight
                } else {
                    Direction::Undirected
                };
                (direction, None, false, Vec::new(), None, None, None)
            };
            return Ok(RelationshipPattern {
                direction,
                variable,
                temporary,
                labels,
                label_expression,
                properties,
                where_clause,
                quantifier: self.parse_optional_path_quantifier()?,
            });
        }

        self.expect(TokenKind::Minus)?;
        let has_body = self.eat(TokenKind::LBracket).is_some();
        let (variable, temporary, labels, label_expression, properties, where_clause) = if has_body
        {
            let body = self.parse_pattern_element_body(TokenKind::RBracket)?;
            self.expect(TokenKind::RBracket)?;
            body
        } else {
            (None, false, Vec::new(), None, None, None)
        };

        let (direction, quantifier) = if self.eat(TokenKind::ArrowRight).is_some() {
            (Direction::Right, self.parse_optional_path_quantifier()?)
        } else if self.eat(TokenKind::Minus).is_some() {
            (Direction::Any, self.parse_optional_path_quantifier()?)
        } else if has_body {
            return Err(self.error_expected(
                vec![TokenKind::ArrowRight, TokenKind::Minus],
                self.peek_kind(),
            ));
        } else {
            (Direction::Any, self.parse_optional_path_quantifier()?)
        };

        Ok(RelationshipPattern {
            direction,
            variable,
            temporary,
            labels,
            label_expression,
            properties,
            where_clause,
            quantifier,
        })
    }

    fn parse_insert_relationship_pattern(&mut self) -> Result<RelationshipPattern> {
        if self.eat(TokenKind::ArrowLeft).is_some() {
            self.expect(TokenKind::LBracket)?;
            let (variable, labels, label_expression, properties) = if self.at(TokenKind::RBracket) {
                (None, Vec::new(), None, None)
            } else {
                self.parse_insert_element_pattern_body(TokenKind::RBracket)?
            };
            self.expect(TokenKind::RBracket)?;
            self.expect(TokenKind::Minus)?;
            return Ok(RelationshipPattern {
                direction: Direction::Left,
                variable,
                temporary: false,
                labels,
                label_expression,
                properties,
                where_clause: None,
                quantifier: None,
            });
        }

        if self.eat(TokenKind::Tilde).is_some() {
            self.expect(TokenKind::LBracket)?;
            let (variable, labels, label_expression, properties) = if self.at(TokenKind::RBracket) {
                (None, Vec::new(), None, None)
            } else {
                self.parse_insert_element_pattern_body(TokenKind::RBracket)?
            };
            self.expect(TokenKind::RBracket)?;
            self.expect(TokenKind::Tilde)?;
            return Ok(RelationshipPattern {
                direction: Direction::Undirected,
                variable,
                temporary: false,
                labels,
                label_expression,
                properties,
                where_clause: None,
                quantifier: None,
            });
        }

        self.expect(TokenKind::Minus)?;
        self.expect(TokenKind::LBracket)?;
        let (variable, labels, label_expression, properties) = if self.at(TokenKind::RBracket) {
            (None, Vec::new(), None, None)
        } else {
            self.parse_insert_element_pattern_body(TokenKind::RBracket)?
        };
        self.expect(TokenKind::RBracket)?;
        let direction = if self.eat(TokenKind::ArrowRight).is_some() {
            Direction::Right
        } else {
            self.expect(TokenKind::Minus)?;
            Direction::Undirected
        };
        Ok(RelationshipPattern {
            direction,
            variable,
            temporary: false,
            labels,
            label_expression,
            properties,
            where_clause: None,
            quantifier: None,
        })
    }

    fn parse_pattern_element_body(&mut self, terminator: TokenKind) -> Result<PatternElementBody> {
        let temporary =
            self.at(TokenKind::Temp) && self.peek_n_kind(1).is_some_and(is_identifier_like);
        if temporary {
            return Err(Error::Message {
                offset: self.peek().offset,
                message:
                    "TEMP element variable declarations are not user-visible standard GQL syntax"
                        .to_owned(),
            });
        }

        let variable = if self.starts_element_variable() {
            Some(self.parse_identifier()?)
        } else {
            None
        };

        let mut labels = Vec::new();
        let mut label_expression = None;
        while self.eat(TokenKind::Colon).is_some() || self.eat(TokenKind::Is).is_some() {
            let expr = self.parse_label_expression()?;
            collect_label_names(&expr, &mut labels);
            label_expression = Some(match label_expression {
                Some(existing) => LabelExpression::And(Box::new(existing), Box::new(expr)),
                None => expr,
            });
        }

        let (properties, where_clause) = if self.at(TokenKind::LBrace) {
            (Some(self.parse_map_literal()?), None)
        } else if self.eat(TokenKind::Where).is_some() {
            (None, Some(self.parse_expr()?))
        } else {
            (None, None)
        };

        if !self.at(terminator) {
            return Err(self.error_expected(vec![terminator], self.peek_kind()));
        }
        Ok((
            variable,
            temporary,
            labels,
            label_expression,
            properties,
            where_clause,
        ))
    }

    fn parse_insert_element_pattern_body(
        &mut self,
        terminator: TokenKind,
    ) -> Result<InsertElementBody> {
        let variable = if self.starts_element_variable()
            && !matches!(self.peek_n_kind(1), Some(TokenKind::Ampersand) | None)
        {
            Some(self.parse_identifier()?)
        } else {
            None
        };

        let (labels, label_expression) =
            if self.eat(TokenKind::Colon).is_some() || self.eat(TokenKind::Is).is_some() {
                let expr = self.parse_label_expression()?;
                let mut labels = Vec::new();
                collect_label_names(&expr, &mut labels);
                (labels, Some(expr))
            } else if is_identifier_like(self.peek_kind()) {
                self.parse_insert_label_set()?
            } else {
                (Vec::new(), None)
            };

        let properties = if self.at(TokenKind::LBrace) {
            Some(self.parse_map_literal()?)
        } else {
            None
        };

        if !self.at(terminator) {
            return Err(self.error_expected(vec![terminator], self.peek_kind()));
        }
        Ok((variable, labels, label_expression, properties))
    }

    fn starts_element_variable(&self) -> bool {
        is_identifier_like(self.peek_kind())
            && !matches!(self.peek_kind(), TokenKind::Is | TokenKind::Where)
    }

    fn parse_insert_label_set(&mut self) -> Result<(Vec<Identifier>, Option<LabelExpression>)> {
        let mut labels = Vec::new();
        let label = self.parse_identifier()?;
        push_identifier_if_absent(&mut labels, Some(&label));
        while self.eat(TokenKind::Ampersand).is_some() {
            let label = self.parse_identifier()?;
            push_identifier_if_absent(&mut labels, Some(&label));
        }

        let mut iter = labels.iter().cloned();
        let first = iter
            .next()
            .ok_or_else(|| self.error_expected(vec![TokenKind::Identifier], self.peek_kind()))?;
        let mut expr = LabelExpression::Label(first);
        for label in iter {
            expr = LabelExpression::And(Box::new(expr), Box::new(LabelExpression::Label(label)));
        }

        Ok((labels, Some(expr)))
    }

    fn parse_optional_path_quantifier(&mut self) -> Result<Option<PathPatternQuantifier>> {
        if self.eat(TokenKind::Star).is_some() {
            return Ok(Some(PathPatternQuantifier::ZeroOrMore));
        }
        if self.eat(TokenKind::Plus).is_some() {
            return Ok(Some(PathPatternQuantifier::OneOrMore));
        }
        if self.eat(TokenKind::Question).is_some() {
            return Ok(Some(PathPatternQuantifier::Optional));
        }
        if self.eat(TokenKind::LBrace).is_none() {
            return Ok(None);
        }

        let min = if self.at(TokenKind::Integer) {
            Some(self.parse_u64_literal()?)
        } else {
            None
        };
        let quantifier = if self.eat(TokenKind::Comma).is_some() {
            let max = if self.at(TokenKind::Integer) {
                Some(self.parse_u64_literal()?)
            } else {
                None
            };
            PathPatternQuantifier::Range { min, max }
        } else {
            let value = min.ok_or_else(|| {
                self.error_expected(vec![TokenKind::Integer, TokenKind::Comma], self.peek_kind())
            })?;
            PathPatternQuantifier::Fixed(value)
        };
        self.expect(TokenKind::RBrace)?;
        Ok(Some(quantifier))
    }

    fn parse_u64_literal(&mut self) -> Result<u64> {
        let token = self.expect(TokenKind::Integer)?;
        token.text.parse::<u64>().map_err(|_| Error::BadNumber {
            offset: token.offset,
            token_text: token.text,
        })
    }

    fn parse_positive_u64_literal(&mut self, description: &str) -> Result<u64> {
        let value = self.parse_u64_literal()?;
        self.ensure_positive_u64(value, description)?;
        Ok(value)
    }

    fn ensure_positive_u64(&self, value: u64, description: &str) -> Result<()> {
        if value == 0 {
            return Err(Error::Message {
                offset: self.peek().offset,
                message: format!("{description} must be greater than or equal to 1"),
            });
        }
        Ok(())
    }

    fn parse_label_expression(&mut self) -> Result<LabelExpression> {
        self.parse_label_or()
    }

    fn parse_label_or(&mut self) -> Result<LabelExpression> {
        let mut expr = self.parse_label_and()?;
        while self.eat(TokenKind::Pipe).is_some() {
            let right = self.parse_label_and()?;
            expr = LabelExpression::Or(Box::new(expr), Box::new(right));
        }
        Ok(expr)
    }

    fn parse_label_and(&mut self) -> Result<LabelExpression> {
        let mut expr = self.parse_label_unary()?;
        while self.eat(TokenKind::Ampersand).is_some() {
            let right = self.parse_label_unary()?;
            expr = LabelExpression::And(Box::new(expr), Box::new(right));
        }
        Ok(expr)
    }

    fn parse_label_unary(&mut self) -> Result<LabelExpression> {
        if self.at(TokenKind::Not) {
            return Err(Error::Message {
                offset: self.peek().offset,
                message: "label negation uses !, not NOT".to_owned(),
            });
        }
        if self.eat(TokenKind::Bang).is_some() {
            return Ok(LabelExpression::Not(Box::new(self.parse_label_unary()?)));
        }
        if self.eat(TokenKind::LParen).is_some() {
            let expr = self.parse_label_expression()?;
            self.expect(TokenKind::RParen)?;
            return Ok(LabelExpression::Parenthesized(Box::new(expr)));
        }
        if self.eat(TokenKind::Percent).is_some() {
            return Ok(LabelExpression::Wildcard);
        }
        Ok(LabelExpression::Label(self.parse_identifier()?))
    }

    fn parse_expr(&mut self) -> Result<Expr> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr> {
        let mut expr = self.parse_and()?;
        loop {
            let op = if self.eat(TokenKind::Or).is_some() {
                BinaryOp::Or
            } else if self.eat(TokenKind::Xor).is_some() {
                BinaryOp::Xor
            } else {
                break;
            };
            let right = self.parse_and()?;
            expr = Expr::Binary {
                left: Box::new(expr),
                op,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn parse_and(&mut self) -> Result<Expr> {
        let mut expr = self.parse_comparison()?;
        while self.eat(TokenKind::And).is_some() {
            let right = self.parse_comparison()?;
            expr = Expr::Binary {
                left: Box::new(expr),
                op: BinaryOp::And,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn parse_comparison(&mut self) -> Result<Expr> {
        let mut expr = self.parse_concat()?;
        loop {
            if self.eat(TokenKind::Colon).is_some() {
                let variable = self.expr_to_element_variable(expr, "labeled")?;
                let label_expression = self.parse_label_expression()?;
                expr = Expr::IsLabeled {
                    variable,
                    negated: false,
                    label_expression,
                };
                continue;
            }

            let op = match self.peek_kind() {
                TokenKind::Eq => BinaryOp::Eq,
                TokenKind::Neq => BinaryOp::Neq,
                TokenKind::Lt => BinaryOp::Lt,
                TokenKind::Le => BinaryOp::Le,
                TokenKind::Gt => BinaryOp::Gt,
                TokenKind::Ge => BinaryOp::Ge,
                TokenKind::Is => {
                    self.bump();
                    let negated = self.eat(TokenKind::Not).is_some();
                    expr = self.parse_is_predicate_tail(expr, negated)?;
                    continue;
                }
                _ => break,
            };
            self.bump();
            let right = self.parse_concat()?;
            expr = Expr::Binary {
                left: Box::new(expr),
                op,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn parse_is_predicate_tail(&mut self, expr: Expr, negated: bool) -> Result<Expr> {
        if self.at(TokenKind::Typed) || self.at(TokenKind::DoubleColon) {
            self.parse_optional_typed_marker()?;
            let value_type = self.parse_value_type()?;
            return Ok(Expr::IsTyped {
                expr: Box::new(expr),
                negated,
                value_type,
            });
        }
        if self.eat(TokenKind::Null).is_some() {
            return Ok(Expr::IsNull {
                expr: Box::new(expr),
                negated,
            });
        }
        if self.eat(TokenKind::Unknown).is_some() {
            return Ok(Expr::IsUnknown {
                expr: Box::new(expr),
                negated,
            });
        }
        if self.eat(TokenKind::True).is_some() {
            return Ok(Expr::IsTruth {
                expr: Box::new(expr),
                negated,
                value: true,
            });
        }
        if self.eat(TokenKind::False).is_some() {
            return Ok(Expr::IsTruth {
                expr: Box::new(expr),
                negated,
                value: false,
            });
        }
        if matches!(self.peek_kind(), TokenKind::Source | TokenKind::Destination) {
            let node = self.expr_to_element_variable(expr, "source/destination")?;
            let kind = if self.eat(TokenKind::Source).is_some() {
                SourceDestinationKind::Source
            } else {
                self.expect(TokenKind::Destination)?;
                SourceDestinationKind::Destination
            };
            self.expect(TokenKind::Of)?;
            let edge = self.parse_identifier()?;
            return Ok(Expr::SourceDestination {
                node,
                negated,
                kind,
                edge,
            });
        }
        if self.at(TokenKind::Normalized)
            || (is_identifier_like(self.peek_kind())
                && self.peek_n_kind(1) == Some(TokenKind::Normalized))
        {
            let normal_form = if self.at(TokenKind::Normalized) {
                None
            } else {
                Some(self.parse_normal_form()?)
            };
            self.expect(TokenKind::Normalized)?;
            return Ok(Expr::IsNormalized {
                expr: Box::new(expr),
                negated,
                normal_form,
            });
        }
        if self.eat(TokenKind::Directed).is_some() {
            let variable = self.expr_to_element_variable(expr, "directed")?;
            return Ok(Expr::IsDirected { variable, negated });
        }
        if self.eat(TokenKind::Labeled).is_some() {
            let variable = self.expr_to_element_variable(expr, "labeled")?;
            let label_expression = self.parse_label_expression()?;
            return Ok(Expr::IsLabeled {
                variable,
                negated,
                label_expression,
            });
        }

        Err(Error::Message {
            offset: self.peek().offset,
            message: "expected standard IS predicate tail".to_owned(),
        })
    }

    fn parse_concat(&mut self) -> Result<Expr> {
        let mut expr = self.parse_additive()?;
        while self.eat(TokenKind::DoublePipe).is_some() {
            let right = self.parse_additive()?;
            expr = Expr::Binary {
                left: Box::new(expr),
                op: BinaryOp::Concat,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn parse_additive(&mut self) -> Result<Expr> {
        let mut expr = self.parse_multiplicative()?;
        loop {
            let op = if self.eat(TokenKind::Plus).is_some() {
                BinaryOp::Add
            } else if self.eat(TokenKind::Minus).is_some() {
                BinaryOp::Sub
            } else {
                break;
            };
            let right = self.parse_multiplicative()?;
            expr = Expr::Binary {
                left: Box::new(expr),
                op,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr> {
        let mut expr = self.parse_unary()?;
        loop {
            let op = if self.eat(TokenKind::Star).is_some() {
                BinaryOp::Mul
            } else if self.eat(TokenKind::Slash).is_some() {
                BinaryOp::Div
            } else {
                break;
            };
            let right = self.parse_unary()?;
            expr = Expr::Binary {
                left: Box::new(expr),
                op,
                right: Box::new(right),
            };
        }
        Ok(expr)
    }

    fn parse_unary(&mut self) -> Result<Expr> {
        if self.eat(TokenKind::Not).is_some() {
            return Ok(Expr::Unary {
                op: UnaryOp::Not,
                expr: Box::new(self.parse_unary()?),
            });
        }
        if self.eat(TokenKind::Plus).is_some() {
            return Ok(Expr::Unary {
                op: UnaryOp::Pos,
                expr: Box::new(self.parse_unary()?),
            });
        }
        if self.eat(TokenKind::Minus).is_some() {
            return Ok(Expr::Unary {
                op: UnaryOp::Neg,
                expr: Box::new(self.parse_unary()?),
            });
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr> {
        let mut expr = self.parse_primary()?;
        loop {
            if self.eat(TokenKind::Dot).is_some() {
                let key = self.parse_identifier()?;
                expr = Expr::Property {
                    base: Box::new(expr),
                    key,
                };
                continue;
            }
            break;
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr> {
        let token = self.bump();
        match token.kind {
            TokenKind::Date if self.at(TokenKind::LParen) => {
                self.parse_datetime_value_function(DateTimeFunctionKind::Date)
            }
            TokenKind::Date => self.parse_typed_string_literal(LiteralKind::Date),
            TokenKind::Time => self.parse_typed_string_literal(LiteralKind::Time),
            TokenKind::Datetime | TokenKind::Timestamp => {
                self.parse_typed_string_literal(LiteralKind::DateTime)
            }
            TokenKind::Interval => self.parse_sql_interval_literal(),
            TokenKind::Duration if self.is_character_string_token(self.peek_kind()) => {
                self.parse_typed_string_literal(LiteralKind::Duration)
            }
            TokenKind::Duration if self.at(TokenKind::LParen) => self.parse_duration_function(),
            TokenKind::DurationBetween if self.at(TokenKind::LParen) => {
                self.parse_duration_between_function()
            }
            TokenKind::Record if self.at(TokenKind::LBrace) => self.parse_record_constructor(true),
            TokenKind::List if self.at(TokenKind::LBracket) => {
                self.parse_typed_list_constructor(ListValueTypeName::List)
            }
            TokenKind::Array if self.at(TokenKind::LBracket) => {
                self.parse_typed_list_constructor(ListValueTypeName::Array)
            }
            TokenKind::Group
                if self.at(TokenKind::List) && self.peek_n_kind(1) == Some(TokenKind::LBracket) =>
            {
                Err(Error::Message {
                    offset: token.offset,
                    message: "GROUP list value type names are not user-visible standard GQL syntax"
                        .to_owned(),
                })
            }
            TokenKind::Group
                if self.at(TokenKind::Array)
                    && self.peek_n_kind(1) == Some(TokenKind::LBracket) =>
            {
                Err(Error::Message {
                    offset: token.offset,
                    message: "GROUP list value type names are not user-visible standard GQL syntax"
                        .to_owned(),
                })
            }
            TokenKind::Path if self.at(TokenKind::LBracket) => self.parse_path_value_constructor(),
            TokenKind::Case => self.parse_case_expression(),
            TokenKind::Coalesce if self.at(TokenKind::LParen) => self.parse_coalesce_expression(),
            TokenKind::Nullif if self.at(TokenKind::LParen) => self.parse_nullif_expression(),
            TokenKind::Cast => self.parse_cast_expression(),
            TokenKind::CurrentDate => Ok(Expr::CurrentDate),
            TokenKind::CurrentTime => self.parse_current_time_function(),
            TokenKind::CurrentTimestamp => self.parse_current_timestamp_function(),
            TokenKind::CurrentUser => Ok(Expr::CurrentUser),
            TokenKind::ZonedTime if self.at(TokenKind::LParen) => {
                self.parse_datetime_value_function(DateTimeFunctionKind::ZonedTime)
            }
            TokenKind::ZonedDatetime if self.at(TokenKind::LParen) => {
                self.parse_datetime_value_function(DateTimeFunctionKind::ZonedDateTime)
            }
            TokenKind::LocalTime if self.at(TokenKind::LParen) => {
                self.parse_datetime_value_function(DateTimeFunctionKind::LocalTime)
            }
            TokenKind::LocalTime => Ok(Expr::LocalTime { precision: None }),
            TokenKind::LocalDatetime if self.at(TokenKind::LParen) => {
                self.parse_datetime_value_function(DateTimeFunctionKind::LocalDateTime)
            }
            TokenKind::Localtimestamp => self.parse_local_timestamp_function(),
            TokenKind::Abs if self.at(TokenKind::LParen) => self.parse_absolute_value_function(),
            TokenKind::Floor if self.at(TokenKind::LParen) => {
                self.parse_unary_numeric_function(NumericFunctionKind::Floor)
            }
            TokenKind::Ceil | TokenKind::Ceiling if self.at(TokenKind::LParen) => {
                self.parse_unary_numeric_function(NumericFunctionKind::Ceil)
            }
            TokenKind::Sqrt if self.at(TokenKind::LParen) => {
                self.parse_unary_numeric_function(NumericFunctionKind::Sqrt)
            }
            TokenKind::Log10 if self.at(TokenKind::LParen) => {
                self.parse_unary_numeric_function(NumericFunctionKind::Log10)
            }
            TokenKind::Ln if self.at(TokenKind::LParen) => {
                self.parse_unary_numeric_function(NumericFunctionKind::Ln)
            }
            TokenKind::Exp if self.at(TokenKind::LParen) => {
                self.parse_unary_numeric_function(NumericFunctionKind::Exp)
            }
            TokenKind::Sin if self.at(TokenKind::LParen) => {
                self.parse_unary_numeric_function(NumericFunctionKind::Sin)
            }
            TokenKind::Cos if self.at(TokenKind::LParen) => {
                self.parse_unary_numeric_function(NumericFunctionKind::Cos)
            }
            TokenKind::Tan if self.at(TokenKind::LParen) => {
                self.parse_unary_numeric_function(NumericFunctionKind::Tan)
            }
            TokenKind::Cot if self.at(TokenKind::LParen) => {
                self.parse_unary_numeric_function(NumericFunctionKind::Cot)
            }
            TokenKind::Sinh if self.at(TokenKind::LParen) => {
                self.parse_unary_numeric_function(NumericFunctionKind::Sinh)
            }
            TokenKind::Cosh if self.at(TokenKind::LParen) => {
                self.parse_unary_numeric_function(NumericFunctionKind::Cosh)
            }
            TokenKind::Tanh if self.at(TokenKind::LParen) => {
                self.parse_unary_numeric_function(NumericFunctionKind::Tanh)
            }
            TokenKind::Asin if self.at(TokenKind::LParen) => {
                self.parse_unary_numeric_function(NumericFunctionKind::Asin)
            }
            TokenKind::Acos if self.at(TokenKind::LParen) => {
                self.parse_unary_numeric_function(NumericFunctionKind::Acos)
            }
            TokenKind::Atan if self.at(TokenKind::LParen) => {
                self.parse_unary_numeric_function(NumericFunctionKind::Atan)
            }
            TokenKind::Degrees if self.at(TokenKind::LParen) => {
                self.parse_unary_numeric_function(NumericFunctionKind::Degrees)
            }
            TokenKind::Radians if self.at(TokenKind::LParen) => {
                self.parse_unary_numeric_function(NumericFunctionKind::Radians)
            }
            TokenKind::PathLength if self.at(TokenKind::LParen) => {
                self.parse_unary_numeric_function(NumericFunctionKind::PathLength)
            }
            TokenKind::Mod if self.at(TokenKind::LParen) => {
                self.parse_binary_numeric_function(NumericFunctionKind::Mod)
            }
            TokenKind::Power if self.at(TokenKind::LParen) => {
                self.parse_binary_numeric_function(NumericFunctionKind::Power)
            }
            TokenKind::Log if self.at(TokenKind::LParen) => {
                self.parse_binary_numeric_function(NumericFunctionKind::Log)
            }
            TokenKind::PropertyExists => self.parse_property_exists_predicate(),
            TokenKind::ElementId => self.parse_element_id_function(),
            TokenKind::Value if self.at(TokenKind::LBrace) => {
                self.parse_value_query_expression(token.offset)
            }
            TokenKind::Let if self.starts_let_value_expression() => {
                self.parse_let_value_expression()
            }
            TokenKind::Left if self.at(TokenKind::LParen) => {
                self.parse_left_right_substring_function(SubstringSide::Left)
            }
            TokenKind::Right if self.at(TokenKind::LParen) => {
                self.parse_left_right_substring_function(SubstringSide::Right)
            }
            TokenKind::Trim if self.at(TokenKind::LParen) => self.parse_trim_function(),
            TokenKind::Btrim if self.at(TokenKind::LParen) => {
                self.parse_multi_trim_function(TrimSpec::Both)
            }
            TokenKind::Ltrim if self.at(TokenKind::LParen) => {
                self.parse_multi_trim_function(TrimSpec::Leading)
            }
            TokenKind::Rtrim if self.at(TokenKind::LParen) => {
                self.parse_multi_trim_function(TrimSpec::Trailing)
            }
            TokenKind::Elements if self.at(TokenKind::LParen) => self.parse_elements_function(),
            TokenKind::Upper if self.at(TokenKind::LParen) => self.parse_fold_function(true),
            TokenKind::Lower if self.at(TokenKind::LParen) => self.parse_fold_function(false),
            TokenKind::Normalize if self.at(TokenKind::LParen) => self.parse_normalize_function(),
            TokenKind::CharLength | TokenKind::CharacterLength if self.at(TokenKind::LParen) => {
                self.parse_string_length_function(StringLengthUnit::Characters)
            }
            TokenKind::ByteLength | TokenKind::OctetLength if self.at(TokenKind::LParen) => {
                self.parse_string_length_function(StringLengthUnit::Bytes)
            }
            TokenKind::Exists => self.parse_exists_predicate(),
            TokenKind::Same => self.parse_same_predicate(),
            TokenKind::AllDifferent => self.parse_all_different_predicate(),
            TokenKind::Graph if self.is_strict_graph_expression_start(self.peek_kind()) => {
                Ok(Expr::GraphReference {
                    property_graph: false,
                    graph: self.parse_graph_expression()?,
                })
            }
            TokenKind::Property
                if self.at(TokenKind::Graph)
                    && self
                        .peek_n_kind(1)
                        .is_some_and(|kind| self.is_strict_graph_expression_start(kind)) =>
            {
                self.expect(TokenKind::Graph)?;
                Ok(Expr::GraphReference {
                    property_graph: true,
                    graph: self.parse_graph_expression()?,
                })
            }
            TokenKind::Table if self.is_strict_binding_table_expression_start(self.peek_kind()) => {
                Ok(Expr::BindingTableReference {
                    binding: false,
                    table: self.parse_binding_table_expression()?,
                })
            }
            TokenKind::Binding if self.at(TokenKind::Table) => {
                self.expect(TokenKind::Table)?;
                Ok(Expr::BindingTableReference {
                    binding: true,
                    table: self.parse_binding_table_expression()?,
                })
            }
            TokenKind::DoubleQuotedString
                if self.at(TokenKind::Dot) || self.at(TokenKind::LParen) =>
            {
                let ident = Identifier::new(token.text);
                if self.eat(TokenKind::LParen).is_some() {
                    self.parse_function_call(ident, token.offset)
                } else {
                    Ok(Expr::Identifier(ident))
                }
            }
            TokenKind::DoubleQuotedString => Ok(Expr::Literal(Literal::String(token.text))),
            kind if is_identifier_like(kind)
                && !matches!(
                    kind,
                    TokenKind::Null | TokenKind::True | TokenKind::False | TokenKind::Unknown
                ) =>
            {
                let ident = Identifier::new(token.text);
                if self.eat(TokenKind::LParen).is_some() {
                    self.parse_function_call(ident, token.offset)
                } else {
                    Ok(Expr::Identifier(ident))
                }
            }
            TokenKind::Parameter => Ok(Expr::Parameter(
                token.text.trim_start_matches('$').to_owned(),
            )),
            TokenKind::String => Ok(Expr::Literal(Literal::String(token.text))),
            TokenKind::ByteString => self.parse_byte_string_literal(token),
            TokenKind::Integer => {
                let value = token.text.parse::<i64>().map_err(|_| Error::BadNumber {
                    offset: token.offset,
                    token_text: token.text.clone(),
                })?;
                Ok(Expr::Literal(Literal::Integer(value)))
            }
            TokenKind::Decimal => {
                let value = token.text.parse::<f64>().map_err(|_| Error::BadNumber {
                    offset: token.offset,
                    token_text: token.text.clone(),
                })?;
                Ok(Expr::Literal(Literal::Decimal(value)))
            }
            TokenKind::Null => Ok(Expr::Literal(Literal::Null)),
            TokenKind::True => Ok(Expr::Literal(Literal::Boolean(true))),
            TokenKind::False => Ok(Expr::Literal(Literal::Boolean(false))),
            TokenKind::Unknown => Ok(Expr::Literal(Literal::Unknown)),
            TokenKind::Star => Ok(Expr::Wildcard),
            TokenKind::LParen => {
                let expr = self.parse_expr()?;
                self.expect(TokenKind::RParen)?;
                Ok(expr)
            }
            TokenKind::LBracket => {
                let values = self.parse_list_elements_after_lbracket()?;
                Ok(Expr::List(values))
            }
            TokenKind::LBrace => {
                self.pos -= 1;
                self.parse_record_constructor(false)
            }
            TokenKind::Eof => Err(self.eof_error()),
            kind => Err(Error::expected(
                token.offset,
                vec![
                    TokenKind::Identifier,
                    TokenKind::Parameter,
                    TokenKind::String,
                    TokenKind::ByteString,
                    TokenKind::Integer,
                    TokenKind::Decimal,
                    TokenKind::Null,
                    TokenKind::True,
                    TokenKind::False,
                    TokenKind::Unknown,
                    TokenKind::Case,
                    TokenKind::Cast,
                    TokenKind::Interval,
                    TokenKind::PropertyExists,
                    TokenKind::ElementId,
                    TokenKind::Exists,
                    TokenKind::Same,
                    TokenKind::AllDifferent,
                    TokenKind::LParen,
                    TokenKind::LBracket,
                    TokenKind::LBrace,
                ],
                kind,
                token.text,
            )),
        }
    }

    fn parse_byte_string_literal(&self, token: Token) -> Result<Expr> {
        let mut value = Vec::with_capacity(token.text.len() / 2);
        for pair in token.text.as_bytes().as_chunks::<2>().0 {
            let pair = std::str::from_utf8(pair).map_err(|_| Error::Message {
                offset: token.offset,
                message: "invalid byte string literal".to_owned(),
            })?;
            let byte = u8::from_str_radix(pair, 16).map_err(|_| Error::Message {
                offset: token.offset,
                message: "invalid byte string literal".to_owned(),
            })?;
            value.push(byte);
        }
        Ok(Expr::Literal(Literal::Bytes(value)))
    }

    fn is_strict_graph_expression_start(&self, kind: TokenKind) -> bool {
        self.starts_object_expression_primary_special_case(kind)
            || matches!(
                kind,
                TokenKind::Identifier
                    | TokenKind::Parameter
                    | TokenKind::Variable
                    | TokenKind::LParen
                    | TokenKind::Slash
                    | TokenKind::Dot
                    | TokenKind::CurrentGraph
                    | TokenKind::CurrentPropertyGraph
                    | TokenKind::HomeGraph
                    | TokenKind::HomePropertyGraph
            )
    }

    fn is_strict_binding_table_expression_start(&self, kind: TokenKind) -> bool {
        self.starts_object_expression_primary_special_case(kind)
            || matches!(
                kind,
                TokenKind::Identifier
                    | TokenKind::Parameter
                    | TokenKind::Variable
                    | TokenKind::LParen
                    | TokenKind::LBrace
                    | TokenKind::Slash
                    | TokenKind::Dot
            )
    }

    fn starts_object_expression_primary_special_case(&self, kind: TokenKind) -> bool {
        matches!(
            kind,
            TokenKind::String
                | TokenKind::ByteString
                | TokenKind::Integer
                | TokenKind::Decimal
                | TokenKind::Null
                | TokenKind::True
                | TokenKind::False
                | TokenKind::Unknown
                | TokenKind::CurrentDate
                | TokenKind::CurrentTime
                | TokenKind::CurrentTimestamp
                | TokenKind::CurrentUser
                | TokenKind::Localtimestamp
                | TokenKind::LocalTime
                | TokenKind::Case
                | TokenKind::LBrace
                | TokenKind::LBracket
        ) || matches!(
            kind,
            TokenKind::Date | TokenKind::Time | TokenKind::Datetime | TokenKind::Timestamp
        ) && matches!(
            self.peek_n_kind(1),
            Some(
                TokenKind::String
                    | TokenKind::DoubleQuotedString
                    | TokenKind::QuotedString
                    | TokenKind::LParen,
            )
        ) || kind == TokenKind::Interval
            && matches!(
                self.peek_n_kind(1),
                Some(
                    TokenKind::String
                        | TokenKind::DoubleQuotedString
                        | TokenKind::QuotedString
                        | TokenKind::Plus
                        | TokenKind::Minus
                )
            )
            || kind == TokenKind::Duration
                && matches!(
                    self.peek_n_kind(1),
                    Some(
                        TokenKind::String
                            | TokenKind::DoubleQuotedString
                            | TokenKind::QuotedString
                            | TokenKind::LParen,
                    )
                )
            || matches!(
                kind,
                TokenKind::Avg
                    | TokenKind::Count
                    | TokenKind::Max
                    | TokenKind::Min
                    | TokenKind::Sum
                    | TokenKind::CollectList
                    | TokenKind::StddevSamp
                    | TokenKind::StddevPop
                    | TokenKind::PercentileCont
                    | TokenKind::PercentileDisc
                    | TokenKind::Cast
                    | TokenKind::ElementId
            ) && self.peek_n_kind(1) == Some(TokenKind::LParen)
            || matches!(kind, TokenKind::Record | TokenKind::Value)
                && self.peek_n_kind(1) == Some(TokenKind::LBrace)
            || matches!(kind, TokenKind::List | TokenKind::Array | TokenKind::Path)
                && self.peek_n_kind(1) == Some(TokenKind::LBracket)
            || kind == TokenKind::Group
                && matches!(
                    self.peek_n_kind(1),
                    Some(TokenKind::List | TokenKind::Array)
                )
                && self.peek_n_kind(2) == Some(TokenKind::LBracket)
            || kind == TokenKind::Let && self.starts_let_value_expression_after_let()
    }

    fn starts_let_value_expression_after_let(&self) -> bool {
        self.peek_n_kind(1) == Some(TokenKind::Value)
            || self.peek_n_kind(1).is_some_and(is_identifier_like)
                && self.peek_n_kind(2) == Some(TokenKind::Eq)
    }

    fn parse_cast_expression(&mut self) -> Result<Expr> {
        self.expect(TokenKind::LParen)?;
        let expr = self.parse_expr()?;
        self.expect(TokenKind::As)?;
        let value_type = self.parse_value_type()?;
        self.expect(TokenKind::RParen)?;
        Ok(Expr::Cast {
            expr: Box::new(expr),
            value_type,
        })
    }

    fn parse_element_id_function(&mut self) -> Result<Expr> {
        self.expect(TokenKind::LParen)?;
        let variable = self.parse_identifier()?;
        self.expect(TokenKind::RParen)?;
        Ok(Expr::ElementId { variable })
    }

    fn parse_value_query_expression(&mut self, offset: usize) -> Result<Expr> {
        let query = self.parse_nested_query_specification()?;
        validate_value_query_expression(&query, offset)?;
        Ok(Expr::ValueQuery {
            query: Box::new(query),
        })
    }

    fn starts_let_value_expression(&self) -> bool {
        self.at(TokenKind::Value)
            || (is_identifier_like(self.peek_kind()) && self.peek_n_kind(1) == Some(TokenKind::Eq))
    }

    fn parse_let_value_expression(&mut self) -> Result<Expr> {
        let mut items = Vec::new();
        loop {
            let item_offset = self.peek().offset;
            let item = self.parse_let_item_with_in_delimiter(true)?;
            self.push_let_item(&mut items, item, item_offset)?;
            if self.eat(TokenKind::Comma).is_none() {
                break;
            }
        }
        self.expect(TokenKind::In)?;
        let value = self.parse_expr()?;
        self.expect(TokenKind::End)?;
        Ok(Expr::Let {
            items,
            value: Box::new(value),
        })
    }

    fn push_let_item(&self, items: &mut Vec<LetItem>, item: LetItem, offset: usize) -> Result<()> {
        if items.iter().any(|existing| existing.name == item.name) {
            return Err(Error::Message {
                offset,
                message: format!("duplicate let variable '{}'", item.name.value),
            });
        }
        items.push(item);
        Ok(())
    }

    fn parse_record_constructor(&mut self, explicit: bool) -> Result<Expr> {
        Ok(Expr::Record {
            explicit,
            fields: self.parse_map_literal()?,
        })
    }

    fn parse_typed_list_constructor(&mut self, type_name: ListValueTypeName) -> Result<Expr> {
        self.expect(TokenKind::LBracket)?;
        let values = self.parse_list_elements_after_lbracket()?;
        Ok(Expr::TypedList { type_name, values })
    }

    fn parse_path_value_constructor(&mut self) -> Result<Expr> {
        self.expect(TokenKind::LBracket)?;
        if self.at(TokenKind::RBracket) {
            return Err(self.error_expected(
                vec![
                    TokenKind::Identifier,
                    TokenKind::Parameter,
                    TokenKind::LParen,
                    TokenKind::Graph,
                    TokenKind::Binding,
                    TokenKind::Table,
                ],
                self.peek_kind(),
            ));
        }
        let mut values = vec![self.parse_expr()?];
        while self.eat(TokenKind::Comma).is_some() {
            values.push(self.parse_expr()?);
            self.expect(TokenKind::Comma)?;
            values.push(self.parse_expr()?);
        }
        self.expect(TokenKind::RBracket)?;
        Ok(Expr::Path(values))
    }

    fn parse_list_elements_after_lbracket(&mut self) -> Result<Vec<Expr>> {
        let mut values = Vec::new();
        if !self.at(TokenKind::RBracket) {
            loop {
                values.push(self.parse_expr()?);
                if self.eat(TokenKind::Comma).is_none() {
                    break;
                }
            }
        }
        self.expect(TokenKind::RBracket)?;
        Ok(values)
    }

    fn parse_duration_function(&mut self) -> Result<Expr> {
        self.expect(TokenKind::LParen)?;
        let value = self.parse_temporal_function_parameter("DURATION")?;
        self.expect(TokenKind::RParen)?;
        Ok(Expr::DurationFunction {
            value: Box::new(value),
        })
    }

    fn parse_duration_between_function(&mut self) -> Result<Expr> {
        self.expect(TokenKind::LParen)?;
        let left = self.parse_expr()?;
        self.expect(TokenKind::Comma)?;
        let right = self.parse_expr()?;
        self.expect(TokenKind::RParen)?;
        Ok(Expr::DurationBetween {
            left: Box::new(left),
            right: Box::new(right),
        })
    }

    fn parse_current_time_function(&mut self) -> Result<Expr> {
        if self.at(TokenKind::LParen) {
            return Err(Error::Message {
                offset: self.peek().offset,
                message: "CURRENT_TIME does not accept parameters".to_owned(),
            });
        }
        Ok(Expr::CurrentTime { precision: None })
    }

    fn parse_current_timestamp_function(&mut self) -> Result<Expr> {
        if self.at(TokenKind::LParen) {
            return Err(Error::Message {
                offset: self.peek().offset,
                message: "CURRENT_TIMESTAMP does not accept parameters".to_owned(),
            });
        }
        Ok(Expr::CurrentTimestamp { precision: None })
    }

    fn parse_datetime_value_function(&mut self, function: DateTimeFunctionKind) -> Result<Expr> {
        self.expect(TokenKind::LParen)?;
        let value = if self.at(TokenKind::RParen) {
            None
        } else {
            Some(Box::new(
                self.parse_temporal_function_parameter(function.name())?,
            ))
        };
        self.expect(TokenKind::RParen)?;
        Ok(Expr::DateTimeFunction { function, value })
    }

    fn parse_temporal_function_parameter(&mut self, function_name: &str) -> Result<Expr> {
        if self.is_character_string_token(self.peek_kind()) {
            return self.parse_character_string_literal_expr();
        }
        if self.eat(TokenKind::Record).is_some() {
            return self.parse_record_constructor(true);
        }
        if self.at(TokenKind::LBrace) {
            return self.parse_record_constructor(false);
        }
        Err(Error::Message {
            offset: self.peek().offset,
            message: format!(
                "{function_name} requires a string literal or record value constructor parameter"
            ),
        })
    }

    fn parse_local_timestamp_function(&mut self) -> Result<Expr> {
        if self.at(TokenKind::LParen) {
            return Err(Error::Message {
                offset: self.peek().offset,
                message: "LOCAL_TIMESTAMP does not accept parameters".to_owned(),
            });
        }
        Ok(Expr::LocalTimestamp { precision: None })
    }

    fn parse_absolute_value_function(&mut self) -> Result<Expr> {
        self.expect(TokenKind::LParen)?;
        let expr = self.parse_expr()?;
        self.expect(TokenKind::RParen)?;
        Ok(Expr::AbsoluteValue {
            expr: Box::new(expr),
        })
    }

    fn parse_unary_numeric_function(&mut self, function: NumericFunctionKind) -> Result<Expr> {
        self.expect(TokenKind::LParen)?;
        let arg = self.parse_expr()?;
        self.expect(TokenKind::RParen)?;
        Ok(Expr::NumericFunction {
            function,
            args: vec![arg],
        })
    }

    fn parse_binary_numeric_function(&mut self, function: NumericFunctionKind) -> Result<Expr> {
        self.expect(TokenKind::LParen)?;
        let left = self.parse_expr()?;
        self.expect(TokenKind::Comma)?;
        let right = self.parse_expr()?;
        self.expect(TokenKind::RParen)?;
        Ok(Expr::NumericFunction {
            function,
            args: vec![left, right],
        })
    }

    fn parse_left_right_substring_function(&mut self, side: SubstringSide) -> Result<Expr> {
        self.expect(TokenKind::LParen)?;
        let value = self.parse_expr()?;
        self.expect(TokenKind::Comma)?;
        let length = self.parse_expr()?;
        self.expect(TokenKind::RParen)?;
        Ok(Expr::SubstringByLength {
            side,
            value: Box::new(value),
            length: Box::new(length),
        })
    }

    fn parse_trim_function(&mut self) -> Result<Expr> {
        self.expect(TokenKind::LParen)?;
        let specification = self.parse_optional_trim_specification();

        let (trim_character, value) = if specification.is_some() {
            let trim_character = if self.eat(TokenKind::From).is_some() {
                None
            } else {
                let trim_character = self.parse_expr()?;
                self.expect(TokenKind::From)?;
                Some(Box::new(trim_character))
            };
            let value = self.parse_expr()?;
            (trim_character, value)
        } else {
            let first_expr = self.parse_expr()?;
            if self.eat(TokenKind::Comma).is_some() {
                let count = self.parse_expr()?;
                self.expect(TokenKind::RParen)?;
                return Ok(Expr::TrimList {
                    value: Box::new(first_expr),
                    count: Box::new(count),
                });
            } else if self.eat(TokenKind::From).is_some() {
                let value = self.parse_expr()?;
                (Some(Box::new(first_expr)), value)
            } else {
                (None, first_expr)
            }
        };

        self.expect(TokenKind::RParen)?;
        Ok(Expr::Trim {
            specification,
            trim_character,
            value: Box::new(value),
        })
    }

    fn parse_optional_trim_specification(&mut self) -> Option<TrimSpec> {
        if self.eat(TokenKind::Leading).is_some() {
            Some(TrimSpec::Leading)
        } else if self.eat(TokenKind::Trailing).is_some() {
            Some(TrimSpec::Trailing)
        } else if self.eat(TokenKind::Both).is_some() {
            Some(TrimSpec::Both)
        } else {
            None
        }
    }

    fn parse_multi_trim_function(&mut self, specification: TrimSpec) -> Result<Expr> {
        self.expect(TokenKind::LParen)?;
        let value = self.parse_expr()?;
        let trim_characters = if self.eat(TokenKind::Comma).is_some() {
            Some(Box::new(self.parse_expr()?))
        } else {
            None
        };
        self.expect(TokenKind::RParen)?;
        Ok(Expr::MultiTrim {
            specification,
            value: Box::new(value),
            trim_characters,
        })
    }

    fn parse_elements_function(&mut self) -> Result<Expr> {
        self.expect(TokenKind::LParen)?;
        let path = self.parse_expr()?;
        self.expect(TokenKind::RParen)?;
        Ok(Expr::Elements {
            path: Box::new(path),
        })
    }

    fn parse_string_length_function(&mut self, unit: StringLengthUnit) -> Result<Expr> {
        self.expect(TokenKind::LParen)?;
        let value = self.parse_expr()?;
        self.expect(TokenKind::RParen)?;
        Ok(Expr::StringLength {
            unit,
            value: Box::new(value),
        })
    }

    fn parse_fold_function(&mut self, upper: bool) -> Result<Expr> {
        self.expect(TokenKind::LParen)?;
        let value = self.parse_expr()?;
        self.expect(TokenKind::RParen)?;
        Ok(Expr::Fold {
            upper,
            value: Box::new(value),
        })
    }

    fn parse_normalize_function(&mut self) -> Result<Expr> {
        self.expect(TokenKind::LParen)?;
        let value = self.parse_expr()?;
        let normal_form = if self.eat(TokenKind::Comma).is_some() {
            Some(self.parse_normal_form()?)
        } else {
            None
        };
        self.expect(TokenKind::RParen)?;
        Ok(Expr::Normalize {
            value: Box::new(value),
            normal_form,
        })
    }

    fn parse_normal_form(&mut self) -> Result<Identifier> {
        let normal_form = self.parse_identifier()?;
        match normal_form.value.to_ascii_uppercase().as_str() {
            "NFC" | "NFD" | "NFKC" | "NFKD" => Ok(normal_form),
            _ => Err(Error::Message {
                offset: self.peek().offset,
                message: format!("invalid normal form '{}'", normal_form.value),
            }),
        }
    }

    fn parse_function_arguments(&mut self) -> Result<(Option<SetQuantifier>, Vec<Expr>)> {
        let mut args = Vec::new();
        if self.at(TokenKind::RParen) {
            return Ok((None, args));
        }

        let quantifier = if self.eat(TokenKind::Distinct).is_some() {
            Some(SetQuantifier::Distinct)
        } else if self.eat(TokenKind::All).is_some() {
            Some(SetQuantifier::All)
        } else {
            None
        };

        loop {
            args.push(self.parse_expr()?);
            if self.eat(TokenKind::Comma).is_none() {
                break;
            }
        }

        Ok((quantifier, args))
    }

    fn parse_function_call(&mut self, name: Identifier, offset: usize) -> Result<Expr> {
        let (quantifier, args) = self.parse_function_arguments()?;
        self.expect(TokenKind::RParen)?;
        reject_non_standard_value_function(&name, offset)?;
        self.validate_aggregate_function_call(&name, quantifier, &args, offset)?;
        let quantifier = normalize_function_quantifier(&name, quantifier, &args);
        Ok(Expr::Function {
            name,
            quantifier,
            args,
        })
    }

    fn validate_aggregate_function_call(
        &self,
        name: &Identifier,
        quantifier: Option<SetQuantifier>,
        args: &[Expr],
        offset: usize,
    ) -> Result<()> {
        if is_aggregate_function_name(&name.value) && args.iter().any(expr_contains_procedure_body)
        {
            return Err(Error::Message {
                offset,
                message: format!(
                    "aggregate function {} cannot contain a procedure body",
                    name.value
                ),
            });
        }

        match name.value.to_ascii_uppercase().as_str() {
            "COUNT" => {
                if args.len() != 1 {
                    return Err(Error::Message {
                        offset,
                        message: "aggregate function COUNT requires one value expression or *"
                            .to_owned(),
                    });
                }
                if matches!(args[0], Expr::Wildcard) && quantifier.is_some() {
                    return Err(Error::Message {
                        offset,
                        message: "aggregate function COUNT(*) cannot specify a set quantifier"
                            .to_owned(),
                    });
                }
            }
            "AVG" | "MAX" | "MIN" | "SUM" | "COLLECT_LIST" | "STDDEV_SAMP" | "STDDEV_POP" => {
                if args.len() != 1 || matches!(args.first(), Some(Expr::Wildcard)) {
                    return Err(Error::Message {
                        offset,
                        message: format!(
                            "aggregate function {} requires one value expression",
                            name.value
                        ),
                    });
                }
            }
            "PERCENTILE_CONT" | "PERCENTILE_DISC"
                if args.len() != 2 || args.iter().any(|arg| matches!(arg, Expr::Wildcard)) =>
            {
                return Err(Error::Message {
                    offset,
                    message: format!(
                        "aggregate function {} requires two numeric value expressions",
                        name.value
                    ),
                });
            }
            _ => {}
        }
        Ok(())
    }

    fn parse_typed_string_literal(&mut self, kind: LiteralKind) -> Result<Expr> {
        let token = self.parse_character_string_token()?;
        let literal = match kind {
            LiteralKind::Date => Literal::Date(token.text),
            LiteralKind::Time => Literal::Time(token.text),
            LiteralKind::DateTime => Literal::DateTime(token.text),
            LiteralKind::Duration => Literal::Duration(token.text),
        };
        Ok(Expr::Literal(literal))
    }

    fn parse_character_string_literal_expr(&mut self) -> Result<Expr> {
        let token = self.parse_character_string_token()?;
        Ok(Expr::Literal(Literal::String(token.text)))
    }

    fn parse_character_string_token(&mut self) -> Result<Token> {
        if self.is_character_string_token(self.peek_kind()) {
            Ok(self.bump())
        } else if self.at(TokenKind::Eof) {
            Err(self.eof_error())
        } else {
            let got = self.peek().clone();
            Err(Error::expected(
                got.offset,
                vec![
                    TokenKind::String,
                    TokenKind::DoubleQuotedString,
                    TokenKind::QuotedString,
                ],
                got.kind,
                got.text,
            ))
        }
    }

    fn is_character_string_token(&self, kind: TokenKind) -> bool {
        matches!(
            kind,
            TokenKind::String | TokenKind::DoubleQuotedString | TokenKind::QuotedString
        )
    }

    fn parse_sql_interval_literal(&mut self) -> Result<Expr> {
        let sign = if self.eat(TokenKind::Plus).is_some() {
            Some("+")
        } else if self.eat(TokenKind::Minus).is_some() {
            Some("-")
        } else {
            None
        };
        let token = self.parse_character_string_token()?;
        let value = if let Some(sign) = sign {
            format!("{sign}{}", token.text)
        } else {
            token.text
        };
        let qualifier = self.parse_sql_interval_qualifier()?;
        Ok(Expr::Literal(Literal::SqlInterval { value, qualifier }))
    }

    fn parse_sql_interval_qualifier(&mut self) -> Result<SqlIntervalQualifier> {
        let (start, start_kind, start_precision, start_fractional_precision) =
            self.parse_sql_interval_start_field()?;
        let (end, end_precision) = if self.eat(TokenKind::To).is_some() {
            if start_fractional_precision.is_some() {
                return Err(Error::Message {
                    offset: self.peek().offset,
                    message:
                        "SECOND interval qualifier cannot specify fractional precision before TO"
                            .to_owned(),
                });
            }
            let end_offset = self.peek().offset;
            let (end, end_kind, end_precision) = self.parse_sql_interval_end_field()?;
            self.validate_sql_interval_range(start_kind, end_kind, end_offset)?;
            (Some(end), end_precision)
        } else {
            (None, start_fractional_precision)
        };
        Ok(SqlIntervalQualifier {
            start,
            start_precision,
            end,
            end_precision,
        })
    }

    fn parse_sql_interval_start_field(
        &mut self,
    ) -> Result<(Identifier, SqlIntervalFieldKind, Option<u64>, Option<u64>)> {
        let (field, kind) = self.parse_sql_interval_field_name()?;
        let (precision, fractional_precision) = if self.eat(TokenKind::LParen).is_some() {
            let precision = self.parse_u64_literal()?;
            let fractional_precision = if self.eat(TokenKind::Comma).is_some() {
                if kind != SqlIntervalFieldKind::Second {
                    return Err(Error::Message {
                        offset: self.peek().offset,
                        message: "only SECOND interval field can specify fractional precision"
                            .to_owned(),
                    });
                }
                Some(self.parse_u64_literal()?)
            } else {
                None
            };
            self.expect(TokenKind::RParen)?;
            (Some(precision), fractional_precision)
        } else {
            (None, None)
        };
        Ok((field, kind, precision, fractional_precision))
    }

    fn parse_sql_interval_end_field(
        &mut self,
    ) -> Result<(Identifier, SqlIntervalFieldKind, Option<u64>)> {
        let (field, kind) = self.parse_sql_interval_field_name()?;
        let precision = if self.eat(TokenKind::LParen).is_some() {
            if kind != SqlIntervalFieldKind::Second {
                return Err(Error::Message {
                    offset: self.peek().offset,
                    message: "only SECOND end interval field can specify fractional precision"
                        .to_owned(),
                });
            }
            let precision = self.parse_u64_literal()?;
            self.expect(TokenKind::RParen)?;
            Some(precision)
        } else {
            None
        };
        Ok((field, kind, precision))
    }

    fn parse_sql_interval_field_name(&mut self) -> Result<(Identifier, SqlIntervalFieldKind)> {
        let offset = self.peek().offset;
        let field = self.parse_identifier()?;
        let kind = match field.value.to_ascii_uppercase().as_str() {
            "YEAR" => SqlIntervalFieldKind::Year,
            "MONTH" => SqlIntervalFieldKind::Month,
            "DAY" => SqlIntervalFieldKind::Day,
            "HOUR" => SqlIntervalFieldKind::Hour,
            "MINUTE" => SqlIntervalFieldKind::Minute,
            "SECOND" => SqlIntervalFieldKind::Second,
            _ => {
                return Err(Error::Message {
                    offset,
                    message: format!("invalid interval field '{}'", field.value),
                });
            }
        };
        Ok((field, kind))
    }

    fn validate_sql_interval_range(
        &self,
        start: SqlIntervalFieldKind,
        end: SqlIntervalFieldKind,
        offset: usize,
    ) -> Result<()> {
        let valid = matches!(
            (start, end),
            (SqlIntervalFieldKind::Year, SqlIntervalFieldKind::Month)
                | (SqlIntervalFieldKind::Day, SqlIntervalFieldKind::Hour)
                | (SqlIntervalFieldKind::Day, SqlIntervalFieldKind::Minute)
                | (SqlIntervalFieldKind::Day, SqlIntervalFieldKind::Second)
                | (SqlIntervalFieldKind::Hour, SqlIntervalFieldKind::Minute)
                | (SqlIntervalFieldKind::Hour, SqlIntervalFieldKind::Second)
                | (SqlIntervalFieldKind::Minute, SqlIntervalFieldKind::Second)
        );
        if !valid {
            return Err(Error::Message {
                offset,
                message: "invalid SQL interval qualifier range".to_owned(),
            });
        }
        Ok(())
    }

    fn expr_to_element_variable(&self, expr: Expr, predicate: &str) -> Result<Identifier> {
        if let Expr::Identifier(identifier) = expr {
            Ok(identifier)
        } else {
            Err(Error::Message {
                offset: self.peek().offset,
                message: format!("{predicate} predicate target must be an element variable"),
            })
        }
    }

    fn parse_case_expression(&mut self) -> Result<Expr> {
        let operand = if self.at(TokenKind::When) {
            None
        } else {
            Some(Box::new(self.parse_expr()?))
        };

        let mut when_clauses = Vec::new();
        while self.eat(TokenKind::When).is_some() {
            let condition = self.parse_case_when_operand(operand.as_deref())?;
            let mut additional_operands = Vec::new();
            if operand.is_some() {
                while self.eat(TokenKind::Comma).is_some() {
                    additional_operands.push(self.parse_case_when_operand(operand.as_deref())?);
                }
            }
            self.expect(TokenKind::Then)?;
            let result = self.parse_expr()?;
            when_clauses.push(CaseWhenClause {
                condition,
                additional_operands,
                result,
            });
        }

        if when_clauses.is_empty() {
            return Err(self.error_expected(vec![TokenKind::When], self.peek_kind()));
        }

        let else_expr = if self.eat(TokenKind::Else).is_some() {
            Some(Box::new(self.parse_expr()?))
        } else {
            None
        };
        self.expect(TokenKind::End)?;

        Ok(Expr::Case {
            operand,
            when_clauses,
            else_expr,
        })
    }

    fn parse_case_when_operand(&mut self, operand: Option<&Expr>) -> Result<Expr> {
        let Some(operand) = operand else {
            return self.parse_expr();
        };

        let op = match self.peek_kind() {
            TokenKind::Eq => Some(BinaryOp::Eq),
            TokenKind::Neq => Some(BinaryOp::Neq),
            TokenKind::Lt => Some(BinaryOp::Lt),
            TokenKind::Le => Some(BinaryOp::Le),
            TokenKind::Gt => Some(BinaryOp::Gt),
            TokenKind::Ge => Some(BinaryOp::Ge),
            TokenKind::Is => {
                self.bump();
                let negated = self.eat(TokenKind::Not).is_some();
                return self.parse_is_predicate_tail(operand.clone(), negated);
            }
            _ => None,
        };

        if let Some(op) = op {
            self.bump();
            let right = self.parse_concat()?;
            return Ok(Expr::Binary {
                left: Box::new(operand.clone()),
                op,
                right: Box::new(right),
            });
        }

        self.parse_case_when_value_operand()
    }

    fn parse_case_when_value_operand(&mut self) -> Result<Expr> {
        if self.at(TokenKind::LParen) {
            return Err(Error::Message {
                offset: self.peek().offset,
                message: "simple CASE WHEN operand must be a non-parenthesized value primary"
                    .to_owned(),
            });
        }
        self.parse_postfix()
    }

    fn parse_coalesce_expression(&mut self) -> Result<Expr> {
        self.expect(TokenKind::LParen)?;
        let mut args = vec![self.parse_expr()?];
        self.expect(TokenKind::Comma)?;
        args.push(self.parse_expr()?);
        while self.eat(TokenKind::Comma).is_some() {
            args.push(self.parse_expr()?);
        }
        self.expect(TokenKind::RParen)?;
        Ok(Expr::Coalesce(args))
    }

    fn parse_nullif_expression(&mut self) -> Result<Expr> {
        self.expect(TokenKind::LParen)?;
        let left = self.parse_expr()?;
        self.expect(TokenKind::Comma)?;
        let right = self.parse_expr()?;
        self.expect(TokenKind::RParen)?;
        Ok(Expr::NullIf {
            left: Box::new(left),
            right: Box::new(right),
        })
    }

    fn parse_property_exists_predicate(&mut self) -> Result<Expr> {
        self.expect(TokenKind::LParen)?;
        let variable = self.parse_identifier()?;
        self.expect(TokenKind::Comma)?;
        let property = self.parse_identifier()?;
        self.expect(TokenKind::RParen)?;
        Ok(Expr::PropertyExists { variable, property })
    }

    fn parse_exists_predicate(&mut self) -> Result<Expr> {
        if self.eat(TokenKind::LBrace).is_some() {
            let mut trial = self.clone();
            if let Ok(query) = trial.parse_query_statement() {
                if trial.eat(TokenKind::RBrace).is_some() {
                    *self = trial;
                    return Ok(Expr::ExistsQuery {
                        query: Box::new(query),
                    });
                }
            }
            if matches!(self.peek_kind(), TokenKind::Optional | TokenKind::Match) {
                let matches = self.parse_match_statement_block(TokenKind::RBrace)?;
                self.expect(TokenKind::RBrace)?;
                return Ok(Expr::ExistsMatch { matches });
            }
            let patterns = self.parse_pattern_list()?;
            self.expect(TokenKind::RBrace)?;
            Ok(Expr::Exists { patterns })
        } else if self.eat(TokenKind::LParen).is_some() {
            if matches!(self.peek_kind(), TokenKind::Optional | TokenKind::Match) {
                let matches = self.parse_match_statement_block(TokenKind::RParen)?;
                self.expect(TokenKind::RParen)?;
                return Ok(Expr::ExistsMatch { matches });
            }
            let patterns = self.parse_pattern_list()?;
            self.expect(TokenKind::RParen)?;
            Ok(Expr::Exists { patterns })
        } else {
            Err(self.error_expected(vec![TokenKind::LBrace, TokenKind::LParen], self.peek_kind()))
        }
    }

    fn parse_match_statement_block(&mut self, closing: TokenKind) -> Result<Vec<MatchClause>> {
        let mut matches = Vec::new();
        while matches!(self.peek_kind(), TokenKind::Optional | TokenKind::Match) {
            matches.push(self.parse_match_clause()?);
        }
        if matches.is_empty() {
            return Err(self.error_expected(
                vec![TokenKind::Optional, TokenKind::Match],
                self.peek_kind(),
            ));
        }
        if self.at(closing) {
            Ok(matches)
        } else {
            Err(self.error_expected(vec![closing], self.peek_kind()))
        }
    }

    fn parse_same_predicate(&mut self) -> Result<Expr> {
        Ok(Expr::Same {
            variables: self.parse_element_variable_list_with_min_two()?,
        })
    }

    fn parse_all_different_predicate(&mut self) -> Result<Expr> {
        Ok(Expr::AllDifferent {
            variables: self.parse_element_variable_list_with_min_two()?,
        })
    }

    fn parse_element_variable_list_with_min_two(&mut self) -> Result<Vec<Identifier>> {
        self.expect(TokenKind::LParen)?;
        let mut variables = vec![self.parse_identifier()?];
        self.expect(TokenKind::Comma)?;
        variables.push(self.parse_identifier()?);
        while self.eat(TokenKind::Comma).is_some() {
            variables.push(self.parse_identifier()?);
        }
        self.expect(TokenKind::RParen)?;
        Ok(variables)
    }

    fn parse_map_literal(&mut self) -> Result<MapLiteral> {
        self.expect(TokenKind::LBrace)?;
        let mut entries = Vec::new();
        let mut field_names = std::collections::HashSet::new();
        if !self.at(TokenKind::RBrace) {
            loop {
                let key_offset = self.peek().offset;
                let key = self.parse_identifier()?;
                let normalized_key = key.value.to_ascii_uppercase();
                if !field_names.insert(normalized_key) {
                    return Err(Error::Message {
                        offset: key_offset,
                        message: format!("duplicate field '{}'", key.value),
                    });
                }
                self.expect(TokenKind::Colon)?;
                let value = self.parse_expr()?;
                entries.push((key, value));
                if self.eat(TokenKind::Comma).is_none() {
                    break;
                }
            }
        }
        self.expect(TokenKind::RBrace)?;
        Ok(MapLiteral { entries })
    }

    fn parse_identifier(&mut self) -> Result<Identifier> {
        if is_identifier_like(self.peek_kind()) {
            Ok(Identifier::new(self.bump().text))
        } else if self.at(TokenKind::Eof) {
            Err(self.eof_error())
        } else {
            let got = self.peek().clone();
            Err(Error::expected(
                got.offset,
                vec![TokenKind::Identifier],
                got.kind,
                got.text,
            ))
        }
    }

    fn expect(&mut self, kind: TokenKind) -> Result<Token> {
        if self.at(kind) {
            Ok(self.bump())
        } else if self.at(TokenKind::Eof) {
            Err(self.eof_error())
        } else {
            let got = self.peek().clone();
            Err(Error::expected(got.offset, vec![kind], got.kind, got.text))
        }
    }

    fn eat(&mut self, kind: TokenKind) -> Option<Token> {
        if self.at(kind) {
            Some(self.bump())
        } else {
            None
        }
    }

    fn eat_many(&mut self, kind: TokenKind) {
        while self.at(kind) {
            self.bump();
        }
    }

    fn bump(&mut self) -> Token {
        let token = self.peek().clone();
        if !self.at(TokenKind::Eof) {
            self.pos += 1;
        }
        token
    }

    fn at(&self, kind: TokenKind) -> bool {
        self.peek_kind() == kind
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn peek_kind(&self) -> TokenKind {
        self.peek().kind
    }

    fn peek_n_kind(&self, n: usize) -> Option<TokenKind> {
        self.tokens.get(self.pos + n).map(|token| token.kind)
    }

    fn error_expected(&self, expected: Vec<TokenKind>, got: TokenKind) -> Error {
        if got == TokenKind::Eof {
            if let Some(err) = self.lexer_error.clone() {
                return err;
            }
        }
        let token = self.peek();
        Error::expected(token.offset, expected, got, token.text.clone())
    }

    fn eof_error(&self) -> Error {
        self.lexer_error.clone().unwrap_or(Error::UnexpectedEof)
    }
}

fn apply_path_pattern_factor_to_term(
    factor: &PathPatternFactor,
    start: &mut Option<NodePattern>,
    chains: &mut Vec<PathPatternChain>,
    pending_relationship: &mut Option<RelationshipPattern>,
) {
    match factor {
        PathPatternFactor::Relationship(relationship) => {
            if let Some(pending) = pending_relationship.take() {
                if start.is_none() {
                    *start = Some(empty_node_pattern());
                }
                chains.push(PathPatternChain {
                    relationship: pending,
                    node: empty_node_pattern(),
                });
            }
            *pending_relationship = Some(relationship.clone());
        }
        PathPatternFactor::Node(node) => {
            apply_path_pattern_node_to_term(node.clone(), start, chains, pending_relationship);
        }
        PathPatternFactor::Parenthesized(parenthesized) => {
            apply_path_pattern_node_to_term(
                parenthesized.pattern.start.clone(),
                start,
                chains,
                pending_relationship,
            );
        }
        PathPatternFactor::Alternation { alternatives, .. } => {
            if let Some(first) = alternatives.first() {
                for factor in first {
                    apply_path_pattern_factor_to_term(factor, start, chains, pending_relationship);
                }
            }
        }
    }
}

fn apply_path_pattern_node_to_term(
    node: NodePattern,
    start: &mut Option<NodePattern>,
    chains: &mut Vec<PathPatternChain>,
    pending_relationship: &mut Option<RelationshipPattern>,
) {
    if let Some(relationship) = pending_relationship.take() {
        if start.is_none() {
            *start = Some(empty_node_pattern());
        }
        chains.push(PathPatternChain { relationship, node });
    } else if start.is_none() {
        *start = Some(node);
    }
}

fn path_pattern_factors_from_flat_path(
    start: &NodePattern,
    chains: &[PathPatternChain],
) -> Vec<PathPatternFactor> {
    let mut factors = vec![PathPatternFactor::Node(start.clone())];
    for chain in chains {
        factors.push(PathPatternFactor::Relationship(chain.relationship.clone()));
        factors.push(PathPatternFactor::Node(chain.node.clone()));
    }
    factors
}

fn empty_node_pattern() -> NodePattern {
    NodePattern {
        variable: None,
        temporary: false,
        labels: Vec::new(),
        label_expression: None,
        properties: None,
        where_clause: None,
        quantifier: None,
    }
}

fn validate_graph_pattern_yield_clause(
    patterns: &[PathPattern],
    yield_clause: &YieldClause,
    offset: usize,
) -> Result<()> {
    let mut variables = Vec::new();
    for pattern in patterns {
        collect_path_pattern_variables(pattern, &mut variables);
    }

    for item in &yield_clause.items {
        let YieldItem::Item { name, .. } = item else {
            continue;
        };
        if !variables
            .iter()
            .any(|variable: &Identifier| variable.value.eq_ignore_ascii_case(&name.value))
        {
            return Err(Error::Message {
                offset,
                message: format!(
                    "graph pattern yield variable '{}' is not declared",
                    name.value
                ),
            });
        }
    }
    Ok(())
}

fn validate_insert_graph_pattern_variables(patterns: &[PathPattern], offset: usize) -> Result<()> {
    let mut declarations = Vec::new();
    for pattern in patterns {
        validate_insert_node_pattern_variable(&pattern.start, &mut declarations, offset)?;
        for chain in &pattern.chains {
            validate_insert_relationship_pattern_variable(
                &chain.relationship,
                &mut declarations,
                offset,
            )?;
            validate_insert_node_pattern_variable(&chain.node, &mut declarations, offset)?;
        }
    }
    Ok(())
}

fn validate_insert_node_pattern_variable(
    node: &NodePattern,
    declarations: &mut Vec<(Identifier, InsertElementKind, bool)>,
    offset: usize,
) -> Result<()> {
    validate_insert_element_variable(
        node.variable.as_ref(),
        InsertElementKind::Node,
        insert_node_pattern_has_label_or_property_set(node),
        declarations,
        offset,
    )
}

fn validate_insert_relationship_pattern_variable(
    relationship: &RelationshipPattern,
    declarations: &mut Vec<(Identifier, InsertElementKind, bool)>,
    offset: usize,
) -> Result<()> {
    validate_insert_element_variable(
        relationship.variable.as_ref(),
        InsertElementKind::Edge,
        insert_relationship_pattern_has_label_or_property_set(relationship),
        declarations,
        offset,
    )
}

fn validate_insert_element_variable(
    variable: Option<&Identifier>,
    kind: InsertElementKind,
    has_label_or_property_set: bool,
    declarations: &mut Vec<(Identifier, InsertElementKind, bool)>,
    offset: usize,
) -> Result<()> {
    let Some(variable) = variable else {
        return Ok(());
    };

    let duplicate = declarations
        .iter()
        .find(|(existing, _, _)| existing.value.eq_ignore_ascii_case(&variable.value));
    if let Some((_, existing_kind, _)) = duplicate {
        if kind == InsertElementKind::Edge || *existing_kind == InsertElementKind::Edge {
            return Err(Error::Message {
                offset,
                message: format!(
                    "insert edge variable '{}' cannot duplicate another element variable",
                    variable.value
                ),
            });
        }
        if has_label_or_property_set {
            return Err(Error::Message {
                offset,
                message: format!(
                    "repeated insert element variable '{}' cannot contain labels or properties",
                    variable.value
                ),
            });
        }
    }

    declarations.push((variable.clone(), kind, has_label_or_property_set));
    Ok(())
}

fn insert_node_pattern_has_label_or_property_set(node: &NodePattern) -> bool {
    !node.labels.is_empty() || node.label_expression.is_some() || node.properties.is_some()
}

fn insert_relationship_pattern_has_label_or_property_set(
    relationship: &RelationshipPattern,
) -> bool {
    !relationship.labels.is_empty()
        || relationship.label_expression.is_some()
        || relationship.properties.is_some()
}

fn validate_set_item_conflicts(existing: &[SetItem], item: &SetItem, offset: usize) -> Result<()> {
    let Some(target) = set_item_assignment_target(item) else {
        return Ok(());
    };

    for existing_item in existing {
        let Some(existing_target) = set_item_assignment_target(existing_item) else {
            continue;
        };
        match (existing_target, target) {
            (
                SetAssignmentTarget::Property {
                    variable: existing_variable,
                    property: existing_property,
                },
                SetAssignmentTarget::Property { variable, property },
            ) if existing_variable.eq_ignore_ascii_case(variable)
                && existing_property.eq_ignore_ascii_case(property) =>
            {
                return Err(Error::Message {
                    offset,
                    message: format!(
                        "duplicate SET assignment to property '{variable}.{property}'"
                    ),
                });
            }
            (
                SetAssignmentTarget::AllProperties {
                    variable: existing_variable,
                },
                SetAssignmentTarget::AllProperties { variable },
            ) if existing_variable.eq_ignore_ascii_case(variable) => {
                return Err(Error::Message {
                    offset,
                    message: format!("duplicate SET all-properties assignment to '{variable}'"),
                });
            }
            (
                SetAssignmentTarget::AllProperties {
                    variable: existing_variable,
                },
                SetAssignmentTarget::Property { variable, .. },
            )
            | (
                SetAssignmentTarget::Property {
                    variable: existing_variable,
                    ..
                },
                SetAssignmentTarget::AllProperties { variable },
            ) if existing_variable.eq_ignore_ascii_case(variable) => {
                return Err(Error::Message {
                    offset,
                    message: format!(
                        "SET all-properties assignment conflicts with property assignment for '{variable}'"
                    ),
                });
            }
            _ => {}
        }
    }

    Ok(())
}

#[derive(Clone, Copy)]
enum SetAssignmentTarget<'a> {
    Property {
        variable: &'a str,
        property: &'a str,
    },
    AllProperties {
        variable: &'a str,
    },
}

fn set_item_assignment_target(item: &SetItem) -> Option<SetAssignmentTarget<'_>> {
    match item {
        SetItem::Property {
            target:
                Expr::Property {
                    base,
                    key: property,
                },
            ..
        } => {
            let Expr::Identifier(variable) = base.as_ref() else {
                return None;
            };
            Some(SetAssignmentTarget::Property {
                variable: variable.value.as_str(),
                property: property.value.as_str(),
            })
        }
        SetItem::AllProperties { variable, .. } => Some(SetAssignmentTarget::AllProperties {
            variable: variable.value.as_str(),
        }),
        SetItem::Label { .. } | SetItem::Property { .. } => None,
    }
}

fn validate_graph_type_element_name(
    existing: &[GraphTypeElement],
    element: &GraphTypeElement,
    offset: usize,
) -> Result<()> {
    let Some((kind, name)) = graph_type_element_kind_and_name(element) else {
        return Ok(());
    };
    let duplicate = existing.iter().any(|existing| {
        graph_type_element_kind_and_name(existing).is_some_and(|(existing_kind, existing_name)| {
            existing_kind == kind && existing_name.eq_ignore_ascii_case(name)
        })
    });
    if duplicate {
        return Err(Error::Message {
            offset,
            message: format!("duplicate {kind} type name '{name}'"),
        });
    }
    Ok(())
}

fn graph_type_element_kind_and_name(element: &GraphTypeElement) -> Option<(&'static str, &str)> {
    match element {
        GraphTypeElement::Node(node) if !node.name.value.is_empty() => {
            Some(("node", node.name.value.as_str()))
        }
        GraphTypeElement::Edge(edge) if !edge.name.value.is_empty() => {
            Some(("edge", edge.name.value.as_str()))
        }
        _ => None,
    }
}

fn validate_graph_type_body_edge_phrase(
    context: GraphTypeElementContext,
    edge_kind: Option<Direction>,
    name: &Identifier,
    offset: usize,
) -> Result<()> {
    if context != GraphTypeElementContext::GraphTypeSpecificationBody {
        return Ok(());
    }
    if edge_kind.is_none() {
        return Err(Error::Message {
            offset,
            message: "graph type body edge type phrase requires DIRECTED or UNDIRECTED".to_owned(),
        });
    }
    if name.value.is_empty() {
        return Err(Error::Message {
            offset,
            message: "graph type body edge type phrase requires an edge type name".to_owned(),
        });
    }
    Ok(())
}

fn validate_closed_edge_reference_definition(
    definition: &EdgeTypeDefinition,
    offset: usize,
) -> Result<()> {
    if !definition.name.value.is_empty() {
        return Err(Error::Message {
            offset,
            message: "closed edge reference type cannot contain an edge type name".to_owned(),
        });
    }
    if definition.source.is_some() || definition.destination.is_some() {
        return Err(Error::Message {
            offset,
            message: "closed edge reference type cannot contain endpoint node type names"
                .to_owned(),
        });
    }
    Ok(())
}

fn validate_endpoint_pair_shape(
    explicit_edge_kind: Option<Direction>,
    endpoint_shape: Direction,
    offset: usize,
) -> Result<()> {
    match (explicit_edge_kind, endpoint_shape) {
        (Some(Direction::Right), Direction::Undirected) => Err(Error::Message {
            offset,
            message: "DIRECTED edge type cannot use an undirected endpoint pair".to_owned(),
        }),
        (Some(Direction::Undirected), Direction::Right) => Err(Error::Message {
            offset,
            message: "UNDIRECTED edge type requires an undirected endpoint pair".to_owned(),
        }),
        _ => Ok(()),
    }
}

fn validate_dynamic_union_component_nullability(
    components: &[ValueType],
    offset: usize,
) -> Result<()> {
    let Some(first) = components.first() else {
        return Ok(());
    };
    let first_is_not_null = is_known_not_null_value_type(first);
    if components
        .iter()
        .any(|component| is_known_not_null_value_type(component) != first_is_not_null)
    {
        return Err(Error::Message {
            offset,
            message: "dynamic union component types must have matching nullability".to_owned(),
        });
    }
    Ok(())
}

fn is_known_not_null_value_type(value_type: &ValueType) -> bool {
    matches!(value_type, ValueType::NotNull(_))
}

fn validate_graph_type_endpoint_node_types(
    elements: &[GraphTypeElement],
    offset: usize,
) -> Result<()> {
    for element in elements {
        let GraphTypeElement::Edge(edge) = element else {
            continue;
        };
        validate_graph_type_endpoint_node_type(elements, edge.source.as_ref(), offset)?;
        validate_graph_type_endpoint_node_type(elements, edge.destination.as_ref(), offset)?;
    }
    Ok(())
}

fn validate_graph_type_endpoint_node_type(
    elements: &[GraphTypeElement],
    endpoint: Option<&Identifier>,
    offset: usize,
) -> Result<()> {
    let Some(endpoint) = endpoint else {
        return Ok(());
    };
    if elements.iter().any(|element| {
        matches!(
            element,
            GraphTypeElement::Node(node)
                if !node.name.value.is_empty()
                    && node.name.value.eq_ignore_ascii_case(&endpoint.value)
        )
    }) {
        return Ok(());
    }
    Err(Error::Message {
        offset,
        message: format!("endpoint node type '{}' is not defined", endpoint.value),
    })
}

fn collect_path_pattern_variables(pattern: &PathPattern, variables: &mut Vec<Identifier>) {
    push_identifier_if_absent(variables, pattern.variable.as_ref());
    collect_node_pattern_variables(&pattern.start, variables);
    for chain in &pattern.chains {
        collect_relationship_pattern_variables(&chain.relationship, variables);
        collect_node_pattern_variables(&chain.node, variables);
    }
    for factor in &pattern.factors {
        collect_path_pattern_factor_variables(factor, variables);
    }
    for alternative in &pattern.alternatives {
        collect_node_pattern_variables(&alternative.start, variables);
        for chain in &alternative.chains {
            collect_relationship_pattern_variables(&chain.relationship, variables);
            collect_node_pattern_variables(&chain.node, variables);
        }
        for factor in &alternative.factors {
            collect_path_pattern_factor_variables(factor, variables);
        }
    }
    if let Some(parenthesized) = &pattern.parenthesized {
        collect_path_pattern_variables(&parenthesized.pattern, variables);
    }
}

fn collect_path_pattern_factor_variables(
    factor: &PathPatternFactor,
    variables: &mut Vec<Identifier>,
) {
    match factor {
        PathPatternFactor::Node(node) => collect_node_pattern_variables(node, variables),
        PathPatternFactor::Relationship(relationship) => {
            collect_relationship_pattern_variables(relationship, variables);
        }
        PathPatternFactor::Parenthesized(parenthesized) => {
            collect_path_pattern_variables(&parenthesized.pattern, variables);
        }
        PathPatternFactor::Alternation { alternatives, .. } => {
            for alternative in alternatives {
                for factor in alternative {
                    collect_path_pattern_factor_variables(factor, variables);
                }
            }
        }
    }
}

fn collect_node_pattern_variables(node: &NodePattern, variables: &mut Vec<Identifier>) {
    if !node.temporary {
        push_identifier_if_absent(variables, node.variable.as_ref());
    }
}

fn collect_relationship_pattern_variables(
    relationship: &RelationshipPattern,
    variables: &mut Vec<Identifier>,
) {
    if !relationship.temporary {
        push_identifier_if_absent(variables, relationship.variable.as_ref());
    }
}

fn push_identifier_if_absent(variables: &mut Vec<Identifier>, candidate: Option<&Identifier>) {
    let Some(candidate) = candidate else {
        return;
    };
    if !variables
        .iter()
        .any(|variable| variable.value.eq_ignore_ascii_case(&candidate.value))
    {
        variables.push(candidate.clone());
    }
}

fn is_statement_follow(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Semicolon | TokenKind::Next | TokenKind::RBrace | TokenKind::Eof
    )
}

impl DateTimeFunctionKind {
    fn name(self) -> &'static str {
        match self {
            DateTimeFunctionKind::Date => "DATE",
            DateTimeFunctionKind::ZonedTime => "ZONED_TIME",
            DateTimeFunctionKind::ZonedDateTime => "ZONED_DATETIME",
            DateTimeFunctionKind::LocalTime => "LOCAL_TIME",
            DateTimeFunctionKind::LocalDateTime => "LOCAL_DATETIME",
        }
    }
}

fn starts_transaction_procedure(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::At
            | TokenKind::Use
            | TokenKind::Match
            | TokenKind::Optional
            | TokenKind::Filter
            | TokenKind::Let
            | TokenKind::For
            | TokenKind::Order
            | TokenKind::Offset
            | TokenKind::Skip
            | TokenKind::Limit
            | TokenKind::LBrace
            | TokenKind::Return
            | TokenKind::Select
            | TokenKind::Finish
            | TokenKind::Call
            | TokenKind::Insert
            | TokenKind::Delete
            | TokenKind::Detach
            | TokenKind::Nodetach
            | TokenKind::Set
            | TokenKind::Remove
            | TokenKind::Create
            | TokenKind::Drop
    )
}

fn is_identifier_like(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Identifier
            | TokenKind::DoubleQuotedString
            | TokenKind::QuotedString
            | TokenKind::Match
            | TokenKind::Optional
            | TokenKind::Where
            | TokenKind::Filter
            | TokenKind::Let
            | TokenKind::For
            | TokenKind::ParameterKeyword
            | TokenKind::Parameters
            | TokenKind::Characteristics
            | TokenKind::In
            | TokenKind::Between
            | TokenKind::Call
            | TokenKind::Yield
            | TokenKind::Next
            | TokenKind::Select
            | TokenKind::Return
            | TokenKind::Distinct
            | TokenKind::All
            | TokenKind::As
            | TokenKind::With
            | TokenKind::Group
            | TokenKind::Groups
            | TokenKind::Having
            | TokenKind::Keep
            | TokenKind::Order
            | TokenKind::By
            | TokenKind::Asc
            | TokenKind::Ascending
            | TokenKind::Desc
            | TokenKind::Descending
            | TokenKind::Nulls
            | TokenKind::First
            | TokenKind::Last
            | TokenKind::Limit
            | TokenKind::Offset
            | TokenKind::Skip
            | TokenKind::Insert
            | TokenKind::Create
            | TokenKind::Delete
            | TokenKind::Set
            | TokenKind::Remove
            | TokenKind::Detach
            | TokenKind::Nodetach
            | TokenKind::Property
            | TokenKind::Graph
            | TokenKind::Schema
            | TokenKind::Type
            | TokenKind::Node
            | TokenKind::Vertex
            | TokenKind::Edge
            | TokenKind::Relationship
            | TokenKind::Relationships
            | TokenKind::From
            | TokenKind::To
            | TokenKind::Connecting
            | TokenKind::If
            | TokenKind::Not
            | TokenKind::Exists
            | TokenKind::Drop
            | TokenKind::Use
            | TokenKind::At
            | TokenKind::Reset
            | TokenKind::Close
            | TokenKind::Session
            | TokenKind::Start
            | TokenKind::Transaction
            | TokenKind::Read
            | TokenKind::Write
            | TokenKind::Only
            | TokenKind::Isolation
            | TokenKind::Level
            | TokenKind::Serializable
            | TokenKind::Repeatable
            | TokenKind::Committed
            | TokenKind::Uncommitted
            | TokenKind::List
            | TokenKind::Array
            | TokenKind::Record
            | TokenKind::Date
            | TokenKind::Time
            | TokenKind::Zone
            | TokenKind::Datetime
            | TokenKind::Timestamp
            | TokenKind::Interval
            | TokenKind::Duration
            | TokenKind::DurationBetween
            | TokenKind::CurrentDate
            | TokenKind::CurrentTime
            | TokenKind::CurrentTimestamp
            | TokenKind::CurrentUser
            | TokenKind::Localtimestamp
            | TokenKind::ZonedTime
            | TokenKind::ZonedDatetime
            | TokenKind::LocalTime
            | TokenKind::LocalDatetime
            | TokenKind::Avg
            | TokenKind::Count
            | TokenKind::Max
            | TokenKind::Min
            | TokenKind::Sum
            | TokenKind::CollectList
            | TokenKind::StddevSamp
            | TokenKind::StddevPop
            | TokenKind::PercentileCont
            | TokenKind::PercentileDisc
            | TokenKind::Abs
            | TokenKind::Mod
            | TokenKind::Floor
            | TokenKind::Ceil
            | TokenKind::Ceiling
            | TokenKind::Sqrt
            | TokenKind::Power
            | TokenKind::Log
            | TokenKind::Log10
            | TokenKind::Ln
            | TokenKind::Exp
            | TokenKind::Sin
            | TokenKind::Cos
            | TokenKind::Tan
            | TokenKind::Cot
            | TokenKind::Sinh
            | TokenKind::Cosh
            | TokenKind::Tanh
            | TokenKind::Asin
            | TokenKind::Acos
            | TokenKind::Atan
            | TokenKind::Degrees
            | TokenKind::Radians
            | TokenKind::PathLength
            | TokenKind::Case
            | TokenKind::Coalesce
            | TokenKind::Nullif
            | TokenKind::Cast
            | TokenKind::When
            | TokenKind::Then
            | TokenKind::Else
            | TokenKind::End
            | TokenKind::PropertyExists
            | TokenKind::ElementId
            | TokenKind::Same
            | TokenKind::AllDifferent
            | TokenKind::Overlay
            | TokenKind::Placing
            | TokenKind::Position
            | TokenKind::Substring
            | TokenKind::Left
            | TokenKind::Right
            | TokenKind::Trim
            | TokenKind::Btrim
            | TokenKind::Ltrim
            | TokenKind::Rtrim
            | TokenKind::Upper
            | TokenKind::Lower
            | TokenKind::Normalize
            | TokenKind::CharLength
            | TokenKind::CharacterLength
            | TokenKind::ByteLength
            | TokenKind::OctetLength
            | TokenKind::Leading
            | TokenKind::Trailing
            | TokenKind::Both
            | TokenKind::Source
            | TokenKind::Destination
            | TokenKind::Of
            | TokenKind::Normalized
            | TokenKind::Directed
            | TokenKind::Undirected
            | TokenKind::Labeled
            | TokenKind::Label
            | TokenKind::Labels
            | TokenKind::Commit
            | TokenKind::Rollback
            | TokenKind::Replace
            | TokenKind::Like
            | TokenKind::Escape
            | TokenKind::Copy
            | TokenKind::Current
            | TokenKind::CurrentGraph
            | TokenKind::CurrentPropertyGraph
            | TokenKind::CurrentSchema
            | TokenKind::HomeGraph
            | TokenKind::HomePropertyGraph
            | TokenKind::HomeSchema
            | TokenKind::And
            | TokenKind::Or
            | TokenKind::Xor
            | TokenKind::Is
            | TokenKind::Typed
            | TokenKind::Null
            | TokenKind::True
            | TokenKind::False
            | TokenKind::Unknown
            | TokenKind::Value
            | TokenKind::Variable
            | TokenKind::Union
            | TokenKind::Except
            | TokenKind::Intersect
            | TokenKind::Otherwise
            | TokenKind::Ordinality
            | TokenKind::Finish
            | TokenKind::No
            | TokenKind::Binding
            | TokenKind::Bindings
            | TokenKind::Table
            | TokenKind::Temp
            | TokenKind::Different
            | TokenKind::Element
            | TokenKind::Elements
            | TokenKind::Edges
            | TokenKind::Any
            | TokenKind::Shortest
            | TokenKind::Walk
            | TokenKind::Trail
            | TokenKind::Simple
            | TokenKind::Acyclic
            | TokenKind::Path
            | TokenKind::Paths
    )
}

fn is_edge_synonym_kind(kind: TokenKind) -> bool {
    matches!(kind, TokenKind::Edge | TokenKind::Relationship)
}

fn is_type_name_after_type_marker(kind: TokenKind) -> bool {
    is_identifier_like(kind)
        && !matches!(
            kind,
            TokenKind::Label
                | TokenKind::Labels
                | TokenKind::LBrace
                | TokenKind::From
                | TokenKind::To
                | TokenKind::Connecting
                | TokenKind::Comma
                | TokenKind::RBrace
                | TokenKind::Eof
        )
}

fn is_data_modifying_query_clause(clause: &QueryClause) -> bool {
    matches!(
        clause,
        QueryClause::Insert(_)
            | QueryClause::Delete(_)
            | QueryClause::Set(_)
            | QueryClause::Remove(_)
    )
}

fn binding_variable_definition_name(definition: &BindingVariableDefinition) -> &Identifier {
    match definition {
        BindingVariableDefinition::Graph(definition) => &definition.name,
        BindingVariableDefinition::BindingTable(definition) => &definition.name,
        BindingVariableDefinition::Value(definition) => &definition.name,
    }
}

fn is_nested_query_clause(clause: &QueryClause) -> bool {
    matches!(clause, QueryClause::NestedQuery(_))
}

fn result_clause_has_wildcard(result_clause: &ResultClause) -> bool {
    result_clause
        .items
        .iter()
        .any(|item| matches!(item.expr, Expr::Wildcard))
}

fn query_clauses_have_non_unit_working_table(clauses: &[QueryClause]) -> bool {
    clauses.iter().any(|clause| {
        matches!(
            clause,
            QueryClause::NestedQuery(_)
                | QueryClause::Match(_)
                | QueryClause::OptionalMatchBlock(_)
                | QueryClause::Let(_)
                | QueryClause::For(_)
                | QueryClause::Call(_)
        )
    })
}

fn validate_value_query_expression(query: &QueryStatement, offset: usize) -> Result<()> {
    let body = &query.body;
    if body.result_clause.items.len() != 1 {
        return Err(Error::Message {
            offset,
            message: "value query expression must return exactly one item".to_owned(),
        });
    }

    if matches!(body.limit, Some(UnsignedIntegerSpecification::Literal(1))) {
        return Ok(());
    }

    if !body.group_by.is_empty() || body.empty_grouping_set {
        return Err(Error::Message {
            offset,
            message: "value query expression without LIMIT 1 cannot contain GROUP BY".to_owned(),
        });
    }

    if expr_contains_aggregate_function(&body.result_clause.items[0].expr) {
        return Ok(());
    }

    Err(Error::Message {
        offset,
        message: "value query expression requires LIMIT 1 or an aggregate result".to_owned(),
    })
}

fn expr_contains_aggregate_function(expr: &Expr) -> bool {
    match expr {
        Expr::Function { name, args, .. } => {
            is_aggregate_function_name(&name.value)
                || args.iter().any(expr_contains_aggregate_function)
        }
        Expr::Property { base, .. } => expr_contains_aggregate_function(base),
        Expr::Unary { expr, .. }
        | Expr::Cast { expr, .. }
        | Expr::DurationFunction { value: expr }
        | Expr::AbsoluteValue { expr }
        | Expr::Elements { path: expr } => expr_contains_aggregate_function(expr),
        Expr::Binary { left, right, .. }
        | Expr::DurationBetween { left, right }
        | Expr::NullIf { left, right } => {
            expr_contains_aggregate_function(left) || expr_contains_aggregate_function(right)
        }
        Expr::SubstringByLength { value, length, .. } => {
            expr_contains_aggregate_function(value) || expr_contains_aggregate_function(length)
        }
        Expr::Trim {
            trim_character,
            value,
            ..
        } => {
            trim_character
                .as_deref()
                .is_some_and(expr_contains_aggregate_function)
                || expr_contains_aggregate_function(value)
        }
        Expr::MultiTrim {
            value,
            trim_characters,
            ..
        } => {
            expr_contains_aggregate_function(value)
                || trim_characters
                    .as_deref()
                    .is_some_and(expr_contains_aggregate_function)
        }
        Expr::TrimList { value, count } => {
            expr_contains_aggregate_function(value) || expr_contains_aggregate_function(count)
        }
        Expr::StringLength { value, .. }
        | Expr::Fold { value, .. }
        | Expr::Normalize { value, .. } => expr_contains_aggregate_function(value),
        Expr::DateTimeFunction { value, .. } => value
            .as_deref()
            .is_some_and(expr_contains_aggregate_function),
        Expr::NumericFunction { args, .. }
        | Expr::Coalesce(args)
        | Expr::List(args)
        | Expr::TypedList { values: args, .. }
        | Expr::Path(args) => args.iter().any(expr_contains_aggregate_function),
        Expr::Let { items, value } => {
            items
                .iter()
                .any(|item| expr_contains_aggregate_function(&item.value))
                || expr_contains_aggregate_function(value)
        }
        Expr::IsTyped { expr, .. }
        | Expr::IsNull { expr, .. }
        | Expr::IsUnknown { expr, .. }
        | Expr::IsTruth { expr, .. }
        | Expr::IsNormalized { expr, .. } => expr_contains_aggregate_function(expr),
        Expr::Case {
            operand,
            when_clauses,
            else_expr,
        } => {
            operand
                .as_deref()
                .is_some_and(expr_contains_aggregate_function)
                || when_clauses.iter().any(|clause| {
                    expr_contains_aggregate_function(&clause.condition)
                        || clause
                            .additional_operands
                            .iter()
                            .any(expr_contains_aggregate_function)
                        || expr_contains_aggregate_function(&clause.result)
                })
                || else_expr
                    .as_deref()
                    .is_some_and(expr_contains_aggregate_function)
        }
        Expr::Record { fields, .. } | Expr::Map(fields) => fields
            .entries
            .iter()
            .any(|(_, value)| expr_contains_aggregate_function(value)),
        Expr::Identifier(_)
        | Expr::GraphReference { .. }
        | Expr::BindingTableReference { .. }
        | Expr::Parameter(_)
        | Expr::Literal(_)
        | Expr::CurrentDate
        | Expr::CurrentTime { .. }
        | Expr::CurrentTimestamp { .. }
        | Expr::CurrentUser
        | Expr::LocalTime { .. }
        | Expr::LocalTimestamp { .. }
        | Expr::ValueQuery { .. }
        | Expr::PropertyExists { .. }
        | Expr::ElementId { .. }
        | Expr::Exists { .. }
        | Expr::ExistsQuery { .. }
        | Expr::ExistsMatch { .. }
        | Expr::Same { .. }
        | Expr::AllDifferent { .. }
        | Expr::SourceDestination { .. }
        | Expr::IsDirected { .. }
        | Expr::IsLabeled { .. }
        | Expr::Wildcard => false,
    }
}

fn expr_references_identifier(expr: &Expr, identifier: &Identifier) -> bool {
    match expr {
        Expr::Identifier(candidate) => candidate.value.eq_ignore_ascii_case(&identifier.value),
        Expr::Property { base, .. } => expr_references_identifier(base, identifier),
        Expr::GraphReference { graph, .. } => {
            graph_expression_references_identifier(graph, identifier)
        }
        Expr::BindingTableReference { table, .. } => {
            binding_table_expression_references_identifier(table, identifier)
        }
        Expr::Unary { expr, .. }
        | Expr::Cast { expr, .. }
        | Expr::DurationFunction { value: expr }
        | Expr::AbsoluteValue { expr }
        | Expr::Elements { path: expr }
        | Expr::IsTyped { expr, .. }
        | Expr::IsNull { expr, .. }
        | Expr::IsUnknown { expr, .. }
        | Expr::IsTruth { expr, .. }
        | Expr::IsNormalized { expr, .. } => expr_references_identifier(expr, identifier),
        Expr::Binary { left, right, .. }
        | Expr::DurationBetween { left, right }
        | Expr::NullIf { left, right } => {
            expr_references_identifier(left, identifier)
                || expr_references_identifier(right, identifier)
        }
        Expr::Function { args, .. }
        | Expr::NumericFunction { args, .. }
        | Expr::Coalesce(args)
        | Expr::List(args)
        | Expr::TypedList { values: args, .. }
        | Expr::Path(args) => args
            .iter()
            .any(|arg| expr_references_identifier(arg, identifier)),
        Expr::SubstringByLength { value, length, .. } => {
            expr_references_identifier(value, identifier)
                || expr_references_identifier(length, identifier)
        }
        Expr::Trim {
            trim_character,
            value,
            ..
        } => {
            trim_character
                .as_deref()
                .is_some_and(|expr| expr_references_identifier(expr, identifier))
                || expr_references_identifier(value, identifier)
        }
        Expr::MultiTrim {
            value,
            trim_characters,
            ..
        } => {
            expr_references_identifier(value, identifier)
                || trim_characters
                    .as_deref()
                    .is_some_and(|expr| expr_references_identifier(expr, identifier))
        }
        Expr::TrimList { value, count } => {
            expr_references_identifier(value, identifier)
                || expr_references_identifier(count, identifier)
        }
        Expr::StringLength { value, .. }
        | Expr::Fold { value, .. }
        | Expr::Normalize { value, .. } => expr_references_identifier(value, identifier),
        Expr::DateTimeFunction { value, .. } => value
            .as_deref()
            .is_some_and(|expr| expr_references_identifier(expr, identifier)),
        Expr::Let { items, value } => {
            items
                .iter()
                .any(|item| expr_references_identifier(&item.value, identifier))
                || expr_references_identifier(value, identifier)
        }
        Expr::PropertyExists { variable, .. }
        | Expr::ElementId { variable }
        | Expr::IsDirected { variable, .. } => {
            variable.value.eq_ignore_ascii_case(&identifier.value)
        }
        Expr::Same { variables } | Expr::AllDifferent { variables } => variables
            .iter()
            .any(|variable| variable.value.eq_ignore_ascii_case(&identifier.value)),
        Expr::SourceDestination { node, edge, .. } => {
            node.value.eq_ignore_ascii_case(&identifier.value)
                || edge.value.eq_ignore_ascii_case(&identifier.value)
        }
        Expr::IsLabeled { variable, .. } => variable.value.eq_ignore_ascii_case(&identifier.value),
        Expr::Case {
            operand,
            when_clauses,
            else_expr,
        } => {
            operand
                .as_deref()
                .is_some_and(|expr| expr_references_identifier(expr, identifier))
                || when_clauses.iter().any(|clause| {
                    expr_references_identifier(&clause.condition, identifier)
                        || clause
                            .additional_operands
                            .iter()
                            .any(|expr| expr_references_identifier(expr, identifier))
                        || expr_references_identifier(&clause.result, identifier)
                })
                || else_expr
                    .as_deref()
                    .is_some_and(|expr| expr_references_identifier(expr, identifier))
        }
        Expr::Record { fields, .. } | Expr::Map(fields) => {
            map_literal_references_identifier(fields, identifier)
        }
        Expr::Parameter(_)
        | Expr::Literal(_)
        | Expr::CurrentDate
        | Expr::CurrentTime { .. }
        | Expr::CurrentTimestamp { .. }
        | Expr::CurrentUser
        | Expr::LocalTime { .. }
        | Expr::LocalTimestamp { .. }
        | Expr::ValueQuery { .. }
        | Expr::Exists { .. }
        | Expr::ExistsQuery { .. }
        | Expr::ExistsMatch { .. }
        | Expr::Wildcard => false,
    }
}

fn graph_expression_references_identifier(
    graph: &GraphExpression,
    identifier: &Identifier,
) -> bool {
    match graph {
        GraphExpression::Variable(expr) => expr_references_identifier(expr, identifier),
        GraphExpression::Name(_)
        | GraphExpression::Parameter(_)
        | GraphExpression::CurrentGraph
        | GraphExpression::CurrentPropertyGraph
        | GraphExpression::HomeGraph
        | GraphExpression::HomePropertyGraph => false,
    }
}

fn binding_table_expression_references_identifier(
    table: &BindingTableExpression,
    identifier: &Identifier,
) -> bool {
    match table {
        BindingTableExpression::Variable(expr) => expr_references_identifier(expr, identifier),
        BindingTableExpression::NestedQuery(_)
        | BindingTableExpression::Name(_)
        | BindingTableExpression::Parameter(_) => false,
    }
}

fn map_literal_references_identifier(map: &MapLiteral, identifier: &Identifier) -> bool {
    map.entries
        .iter()
        .any(|(_, value)| expr_references_identifier(value, identifier))
}

fn expr_contains_procedure_body(expr: &Expr) -> bool {
    match expr {
        Expr::ValueQuery { query } | Expr::ExistsQuery { query } => {
            query_statement_contains_procedure_body(query)
        }
        Expr::GraphReference { graph, .. } => graph_expression_contains_procedure_body(graph),
        Expr::BindingTableReference { table, .. } => {
            binding_table_expression_contains_procedure_body(table)
        }
        Expr::Property { base, .. } => expr_contains_procedure_body(base),
        Expr::Function { args, .. }
        | Expr::NumericFunction { args, .. }
        | Expr::Coalesce(args)
        | Expr::List(args)
        | Expr::TypedList { values: args, .. }
        | Expr::Path(args) => args.iter().any(expr_contains_procedure_body),
        Expr::Unary { expr, .. }
        | Expr::Cast { expr, .. }
        | Expr::DurationFunction { value: expr }
        | Expr::AbsoluteValue { expr }
        | Expr::Elements { path: expr }
        | Expr::IsTyped { expr, .. }
        | Expr::IsNull { expr, .. }
        | Expr::IsUnknown { expr, .. }
        | Expr::IsTruth { expr, .. }
        | Expr::IsNormalized { expr, .. } => expr_contains_procedure_body(expr),
        Expr::Binary { left, right, .. }
        | Expr::DurationBetween { left, right }
        | Expr::NullIf { left, right } => {
            expr_contains_procedure_body(left) || expr_contains_procedure_body(right)
        }
        Expr::SubstringByLength { value, length, .. } => {
            expr_contains_procedure_body(value) || expr_contains_procedure_body(length)
        }
        Expr::Trim {
            trim_character,
            value,
            ..
        } => {
            trim_character
                .as_deref()
                .is_some_and(expr_contains_procedure_body)
                || expr_contains_procedure_body(value)
        }
        Expr::MultiTrim {
            value,
            trim_characters,
            ..
        } => {
            expr_contains_procedure_body(value)
                || trim_characters
                    .as_deref()
                    .is_some_and(expr_contains_procedure_body)
        }
        Expr::TrimList { value, count } => {
            expr_contains_procedure_body(value) || expr_contains_procedure_body(count)
        }
        Expr::StringLength { value, .. }
        | Expr::Fold { value, .. }
        | Expr::Normalize { value, .. } => expr_contains_procedure_body(value),
        Expr::DateTimeFunction { value, .. } => {
            value.as_deref().is_some_and(expr_contains_procedure_body)
        }
        Expr::Let { items, value } => {
            items
                .iter()
                .any(|item| expr_contains_procedure_body(&item.value))
                || expr_contains_procedure_body(value)
        }
        Expr::Case {
            operand,
            when_clauses,
            else_expr,
        } => {
            operand.as_deref().is_some_and(expr_contains_procedure_body)
                || when_clauses.iter().any(|clause| {
                    expr_contains_procedure_body(&clause.condition)
                        || clause
                            .additional_operands
                            .iter()
                            .any(expr_contains_procedure_body)
                        || expr_contains_procedure_body(&clause.result)
                })
                || else_expr
                    .as_deref()
                    .is_some_and(expr_contains_procedure_body)
        }
        Expr::Record { fields, .. } | Expr::Map(fields) => {
            map_literal_contains_procedure_body(fields)
        }
        Expr::Exists { patterns } => patterns.iter().any(path_pattern_contains_procedure_body),
        Expr::ExistsMatch { matches } => matches.iter().any(match_clause_contains_procedure_body),
        Expr::Identifier(_)
        | Expr::Parameter(_)
        | Expr::Literal(_)
        | Expr::CurrentDate
        | Expr::CurrentTime { .. }
        | Expr::CurrentTimestamp { .. }
        | Expr::CurrentUser
        | Expr::LocalTime { .. }
        | Expr::LocalTimestamp { .. }
        | Expr::PropertyExists { .. }
        | Expr::ElementId { .. }
        | Expr::Same { .. }
        | Expr::AllDifferent { .. }
        | Expr::SourceDestination { .. }
        | Expr::IsDirected { .. }
        | Expr::IsLabeled { .. }
        | Expr::Wildcard => false,
    }
}

fn query_statement_contains_procedure_body(query: &QueryStatement) -> bool {
    query
        .use_graph
        .as_ref()
        .is_some_and(graph_expression_contains_procedure_body)
        || query_body_contains_procedure_body(&query.body)
        || query
            .set_operations
            .iter()
            .any(|operation| query_body_contains_procedure_body(&operation.body))
}

fn query_body_contains_procedure_body(body: &QueryBody) -> bool {
    body.clauses
        .iter()
        .any(query_clause_contains_procedure_body)
        || body
            .result_clause
            .items
            .iter()
            .any(|item| expr_contains_procedure_body(&item.expr))
        || body.select_from.iter().any(|item| {
            graph_expression_contains_procedure_body(&item.graph)
                || match_clause_contains_procedure_body(&item.match_clause)
        })
        || body.select_query.as_ref().is_some_and(|select| {
            select
                .graph
                .as_ref()
                .is_some_and(graph_expression_contains_procedure_body)
                || query_statement_contains_procedure_body(&select.query)
        })
        || body
            .select_where
            .as_ref()
            .is_some_and(expr_contains_procedure_body)
        || body.group_by.iter().any(expr_contains_procedure_body)
        || body
            .having
            .as_ref()
            .is_some_and(expr_contains_procedure_body)
        || body
            .order_by
            .iter()
            .any(|item| expr_contains_procedure_body(&item.expr))
}

fn query_clause_contains_procedure_body(clause: &QueryClause) -> bool {
    match clause {
        QueryClause::UseGraph(graph) => graph_expression_contains_procedure_body(graph),
        QueryClause::NestedQuery(query) => query_statement_contains_procedure_body(query),
        QueryClause::Match(match_clause) => match_clause_contains_procedure_body(match_clause),
        QueryClause::OptionalMatchBlock(matches) => {
            matches.iter().any(match_clause_contains_procedure_body)
        }
        QueryClause::Filter(filter) => expr_contains_procedure_body(&filter.predicate),
        QueryClause::Let(let_clause) => let_clause
            .items
            .iter()
            .any(|item| expr_contains_procedure_body(&item.value)),
        QueryClause::For(for_clause) => expr_contains_procedure_body(&for_clause.source),
        QueryClause::Call(call) => call_clause_contains_procedure_body(call),
        QueryClause::Insert(insert) => insert
            .patterns
            .iter()
            .any(path_pattern_contains_procedure_body),
        QueryClause::Delete(delete) => delete.items.iter().any(expr_contains_procedure_body),
        QueryClause::Set(set) => set.items.iter().any(set_item_contains_procedure_body),
        QueryClause::Remove(remove) => remove.items.iter().any(remove_item_contains_procedure_body),
        QueryClause::OrderByPage(page) => page
            .order_by
            .iter()
            .any(|item| expr_contains_procedure_body(&item.expr)),
    }
}

fn call_clause_contains_procedure_body(call: &CallClause) -> bool {
    match &call.call {
        ProcedureCall::Inline(_) => true,
        ProcedureCall::Named { args, .. } => args.iter().any(expr_contains_procedure_body),
    }
}

fn graph_expression_contains_procedure_body(graph: &GraphExpression) -> bool {
    match graph {
        GraphExpression::Variable(expr) => expr_contains_procedure_body(expr),
        GraphExpression::Name(_)
        | GraphExpression::Parameter(_)
        | GraphExpression::CurrentGraph
        | GraphExpression::CurrentPropertyGraph
        | GraphExpression::HomeGraph
        | GraphExpression::HomePropertyGraph => false,
    }
}

fn binding_table_expression_contains_procedure_body(table: &BindingTableExpression) -> bool {
    match table {
        BindingTableExpression::NestedQuery(query) => {
            query_statement_contains_procedure_body(query)
        }
        BindingTableExpression::Variable(expr) => expr_contains_procedure_body(expr),
        BindingTableExpression::Name(_) | BindingTableExpression::Parameter(_) => false,
    }
}

fn match_clause_contains_procedure_body(match_clause: &MatchClause) -> bool {
    match_clause
        .patterns
        .iter()
        .any(path_pattern_contains_procedure_body)
        || match_clause
            .where_clause
            .as_ref()
            .is_some_and(expr_contains_procedure_body)
}

fn path_pattern_contains_procedure_body(pattern: &PathPattern) -> bool {
    node_pattern_contains_procedure_body(&pattern.start)
        || pattern.chains.iter().any(|chain| {
            relationship_pattern_contains_procedure_body(&chain.relationship)
                || node_pattern_contains_procedure_body(&chain.node)
        })
        || pattern
            .factors
            .iter()
            .any(path_pattern_factor_contains_procedure_body)
        || pattern.alternatives.iter().any(|alternative| {
            node_pattern_contains_procedure_body(&alternative.start)
                || alternative.chains.iter().any(|chain| {
                    relationship_pattern_contains_procedure_body(&chain.relationship)
                        || node_pattern_contains_procedure_body(&chain.node)
                })
                || alternative
                    .factors
                    .iter()
                    .any(path_pattern_factor_contains_procedure_body)
        })
        || pattern.parenthesized.as_ref().is_some_and(|parenthesized| {
            path_pattern_contains_procedure_body(&parenthesized.pattern)
                || parenthesized
                    .where_clause
                    .as_ref()
                    .is_some_and(expr_contains_procedure_body)
        })
}

fn path_pattern_factor_contains_procedure_body(factor: &PathPatternFactor) -> bool {
    match factor {
        PathPatternFactor::Node(node) => node_pattern_contains_procedure_body(node),
        PathPatternFactor::Relationship(relationship) => {
            relationship_pattern_contains_procedure_body(relationship)
        }
        PathPatternFactor::Parenthesized(parenthesized) => {
            path_pattern_contains_procedure_body(&parenthesized.pattern)
                || parenthesized
                    .where_clause
                    .as_ref()
                    .is_some_and(expr_contains_procedure_body)
        }
        PathPatternFactor::Alternation { alternatives, .. } => alternatives
            .iter()
            .flatten()
            .any(path_pattern_factor_contains_procedure_body),
    }
}

fn node_pattern_contains_procedure_body(node: &NodePattern) -> bool {
    node.properties
        .as_ref()
        .is_some_and(map_literal_contains_procedure_body)
        || node
            .where_clause
            .as_ref()
            .is_some_and(expr_contains_procedure_body)
}

fn relationship_pattern_contains_procedure_body(relationship: &RelationshipPattern) -> bool {
    relationship
        .properties
        .as_ref()
        .is_some_and(map_literal_contains_procedure_body)
        || relationship
            .where_clause
            .as_ref()
            .is_some_and(expr_contains_procedure_body)
}

fn set_item_contains_procedure_body(item: &SetItem) -> bool {
    match item {
        SetItem::Property { target, value } => {
            expr_contains_procedure_body(target) || expr_contains_procedure_body(value)
        }
        SetItem::AllProperties { properties, .. } => {
            map_literal_contains_procedure_body(properties)
        }
        SetItem::Label { .. } => false,
    }
}

fn remove_item_contains_procedure_body(item: &RemoveItem) -> bool {
    match item {
        RemoveItem::Property(expr) => expr_contains_procedure_body(expr),
        RemoveItem::Label { .. } => false,
    }
}

fn map_literal_contains_procedure_body(map: &MapLiteral) -> bool {
    map.entries
        .iter()
        .any(|(_, value)| expr_contains_procedure_body(value))
}

fn is_aggregate_function_name(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "COUNT"
            | "AVG"
            | "MAX"
            | "MIN"
            | "SUM"
            | "COLLECT_LIST"
            | "STDDEV_SAMP"
            | "STDDEV_POP"
            | "PERCENTILE_CONT"
            | "PERCENTILE_DISC"
    )
}

fn is_general_set_function_name(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "AVG" | "COUNT" | "MAX" | "MIN" | "SUM" | "COLLECT_LIST" | "STDDEV_SAMP" | "STDDEV_POP"
    )
}

fn normalize_function_quantifier(
    name: &Identifier,
    quantifier: Option<SetQuantifier>,
    args: &[Expr],
) -> Option<SetQuantifier> {
    if quantifier.is_none()
        && is_general_set_function_name(&name.value)
        && !matches!(args, [Expr::Wildcard])
    {
        Some(SetQuantifier::All)
    } else {
        quantifier
    }
}

fn reject_non_standard_value_function(name: &Identifier, offset: usize) -> Result<()> {
    if !is_aggregate_function_name(&name.value)
        || matches!(
            name.value.to_ascii_uppercase().as_str(),
            "OVERLAY" | "POSITION" | "SUBSTRING"
        )
    {
        return Err(Error::Message {
            offset,
            message: format!(
                "function {} is not a standard GQL value function",
                name.value
            ),
        });
    }
    Ok(())
}

fn is_schema_reference_follow(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Use
            | TokenKind::Optional
            | TokenKind::Match
            | TokenKind::Filter
            | TokenKind::Let
            | TokenKind::For
            | TokenKind::Call
            | TokenKind::Insert
            | TokenKind::Delete
            | TokenKind::Detach
            | TokenKind::Nodetach
            | TokenKind::Set
            | TokenKind::Remove
            | TokenKind::Order
            | TokenKind::Offset
            | TokenKind::Skip
            | TokenKind::Limit
            | TokenKind::Return
            | TokenKind::Select
            | TokenKind::Finish
            | TokenKind::Semicolon
            | TokenKind::Eof
            | TokenKind::RBrace
    )
}

fn collect_label_names(expr: &LabelExpression, labels: &mut Vec<Identifier>) {
    match expr {
        LabelExpression::Label(label) => labels.push(label.clone()),
        LabelExpression::Wildcard => {}
        LabelExpression::And(left, right) | LabelExpression::Or(left, right) => {
            collect_label_names(left, labels);
            collect_label_names(right, labels);
        }
        LabelExpression::Not(inner) | LabelExpression::Parenthesized(inner) => {
            collect_label_names(inner, labels);
        }
    }
}
