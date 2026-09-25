#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    pub at_schema: Option<SchemaReference>,
    pub definitions: Vec<BindingVariableDefinition>,
    pub statements: Vec<Statement>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BindingVariableDefinition {
    Graph(GraphVariableDefinition),
    BindingTable(BindingTableVariableDefinition),
    Value(ValueVariableDefinition),
}

#[derive(Clone, Debug, PartialEq)]
pub struct GraphVariableDefinition {
    pub property_graph: bool,
    pub name: Identifier,
    pub typed: bool,
    pub value_type: Option<ValueType>,
    pub initializer: GraphExpression,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BindingTableVariableDefinition {
    pub binding: bool,
    pub name: Identifier,
    pub typed: bool,
    pub value_type: Option<ValueType>,
    pub initializer: BindingTableExpression,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ValueVariableDefinition {
    pub name: Identifier,
    pub typed: bool,
    pub value_type: Option<ValueType>,
    pub initializer: Expr,
}

#[derive(Clone, Debug, PartialEq)]
// Keep the public AST's owned variants stable. Boxing is a separate API change.
#[allow(clippy::large_enum_variant)]
pub enum Statement {
    LinearCatalog(Vec<Statement>),
    Query(QueryStatement),
    Call(CallStatement),
    Next(NextStatement),
    Insert(InsertStatement),
    Delete(DeleteStatement),
    Set(SetStatement),
    Remove(RemoveStatement),
    CreateGraph(CreateGraphStatement),
    DropGraph(DropGraphStatement),
    CreateGraphType(CreateGraphTypeStatement),
    DropGraphType(DropGraphTypeStatement),
    CreateSchema(CreateSchemaStatement),
    DropSchema(DropSchemaStatement),
    SessionSet(SessionSetCommand),
    SessionReset(SessionResetCommand),
    SessionClose(SessionCloseCommand),
    StartTransaction(StartTransactionStatement),
    Commit(CommitStatement),
    Rollback(RollbackStatement),
}

#[derive(Clone, Debug, PartialEq)]
pub struct NextStatement {
    pub yield_clause: Option<YieldClause>,
    pub statement: Box<Statement>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueryStatement {
    pub at_schema: Option<SchemaReference>,
    pub use_graph: Option<GraphExpression>,
    pub body: QueryBody,
    pub set_operations: Vec<QuerySetOperation>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueryBody {
    pub clauses: Vec<QueryClause>,
    pub result_clause: ResultClause,
    pub select_from: Vec<SelectGraphMatch>,
    pub select_query: Option<SelectQuerySpecification>,
    pub select_where: Option<Expr>,
    pub group_by: Vec<Expr>,
    pub empty_grouping_set: bool,
    pub having: Option<Expr>,
    pub order_by: Vec<OrderByItem>,
    pub offset: Option<UnsignedIntegerSpecification>,
    pub limit: Option<UnsignedIntegerSpecification>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct QuerySetOperation {
    pub operator: QuerySetOperator,
    pub quantifier: Option<SetQuantifier>,
    pub body: QueryBody,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuerySetOperator {
    Union,
    Except,
    Intersect,
    Otherwise,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetQuantifier {
    Distinct,
    All,
}

#[derive(Clone, Debug, PartialEq)]
pub enum QueryClause {
    UseGraph(GraphExpression),
    NestedQuery(Box<QueryStatement>),
    Match(MatchClause),
    OptionalMatchBlock(Vec<MatchClause>),
    Filter(FilterClause),
    Let(LetClause),
    For(ForClause),
    Call(CallClause),
    Insert(InsertStatement),
    Delete(DeleteStatement),
    Set(SetStatement),
    Remove(RemoveStatement),
    OrderByPage(OrderByPageClause),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SelectGraphMatch {
    pub graph: GraphExpression,
    pub match_clause: MatchClause,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SelectQuerySpecification {
    pub graph: Option<GraphExpression>,
    pub query: Box<QueryStatement>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MatchClause {
    pub optional: bool,
    pub mode: Option<MatchMode>,
    pub patterns: Vec<PathPattern>,
    pub keep: Option<PathPatternPrefix>,
    pub where_clause: Option<Expr>,
    pub yield_clause: Option<YieldClause>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MatchMode {
    RepeatableElements { bindings: bool },
    DifferentEdges { bindings: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub struct FilterClause {
    pub predicate: Expr,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LetClause {
    pub items: Vec<LetItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LetItem {
    pub name: Identifier,
    pub typed: bool,
    pub value_type: Option<ValueType>,
    pub value: Expr,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ForClause {
    pub variable: Identifier,
    pub source: Expr,
    pub ordinality_or_offset: Option<ForOrdinalityOrOffset>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ForOrdinalityOrOffset {
    pub kind: ForOrdinalityOrOffsetKind,
    pub variable: Identifier,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForOrdinalityOrOffsetKind {
    Ordinality,
    Offset,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CallClause {
    pub optional: bool,
    pub call: ProcedureCall,
    pub yield_clause: Option<YieldClause>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ProcedureCall {
    Named {
        procedure: ProcedureReference,
        args: Vec<Expr>,
    },
    Inline(InlineProcedureCall),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcedureReference {
    Name(ProcedureName),
    Parameter(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct InlineProcedureCall {
    pub variable_scope: Option<Vec<Identifier>>,
    pub body: Box<Program>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OrderByPageClause {
    pub order_by: Vec<OrderByItem>,
    pub offset: Option<UnsignedIntegerSpecification>,
    pub limit: Option<UnsignedIntegerSpecification>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CallStatement {
    pub call: CallClause,
}

#[derive(Clone, Debug, PartialEq)]
pub struct YieldClause {
    pub items: Vec<YieldItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum YieldItem {
    All,
    Item {
        name: Identifier,
        alias: Option<Identifier>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResultClause {
    pub kind: ResultKind,
    pub quantifier: Option<SetQuantifier>,
    pub distinct: bool,
    pub items: Vec<ResultItem>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResultKind {
    Return,
    Select,
    Finish,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ResultItem {
    pub expr: Expr,
    pub alias: Option<Identifier>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OrderByItem {
    pub expr: Expr,
    pub direction: Option<SortDirection>,
    pub null_ordering: Option<NullOrdering>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NullOrdering {
    First,
    Last,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InsertStatement {
    pub patterns: Vec<PathPattern>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeleteStatement {
    pub detach: bool,
    pub items: Vec<Expr>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SetStatement {
    pub items: Vec<SetItem>,
}

#[derive(Clone, Debug, PartialEq)]
// Keep the public AST's owned variants stable. Boxing is a separate API change.
#[allow(clippy::large_enum_variant)]
pub enum SetItem {
    Property {
        target: Expr,
        value: Expr,
    },
    AllProperties {
        variable: Identifier,
        properties: MapLiteral,
    },
    Label {
        variable: Identifier,
        label: Identifier,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct RemoveStatement {
    pub items: Vec<RemoveItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RemoveItem {
    Property(Expr),
    Label {
        variable: Identifier,
        label: Identifier,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct SessionSetCommand {
    pub target: SessionSetTarget,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SessionSetTarget {
    Schema(SchemaReference),
    Graph(GraphExpression),
    TimeZone(Expr),
    GraphParameter(SessionSetGraphParameter),
    BindingTableParameter(SessionSetBindingTableParameter),
    ValueParameter(SessionSetValueParameter),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SessionSetGraphParameter {
    pub property_graph: bool,
    pub if_not_exists: bool,
    pub name: String,
    pub typed: bool,
    pub value_type: Option<ValueType>,
    pub initializer: GraphExpression,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SessionSetBindingTableParameter {
    pub binding: bool,
    pub if_not_exists: bool,
    pub name: String,
    pub typed: bool,
    pub value_type: Option<ValueType>,
    pub initializer: BindingTableExpression,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SessionSetValueParameter {
    pub if_not_exists: bool,
    pub name: String,
    pub typed: bool,
    pub value_type: Option<ValueType>,
    pub initializer: Expr,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionResetCommand {
    pub target: SessionResetTarget,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionResetTarget {
    AllCharacteristics,
    AllParameters,
    Schema,
    Graph,
    TimeZone,
    Parameter(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionCloseCommand;

#[derive(Clone, Debug, PartialEq)]
pub struct CreateGraphStatement {
    pub if_not_exists: bool,
    pub or_replace: bool,
    pub property_graph: bool,
    pub name: GraphName,
    pub graph_type: Option<CreateGraphType>,
    pub source: Option<GraphExpression>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropGraphStatement {
    pub if_exists: bool,
    pub property_graph: bool,
    pub name: GraphName,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CreateGraphTypeStatement {
    pub if_not_exists: bool,
    pub or_replace: bool,
    pub property_graph: bool,
    pub name: GraphTypeName,
    pub source: CreateGraphTypeSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropGraphTypeStatement {
    pub if_exists: bool,
    pub property_graph: bool,
    pub name: GraphTypeName,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CreateSchemaStatement {
    pub if_not_exists: bool,
    pub name: SchemaName,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropSchemaStatement {
    pub if_exists: bool,
    pub name: SchemaName,
}

#[derive(Clone, Debug, PartialEq)]
pub enum CreateGraphType {
    Any {
        typed: bool,
        property_graph: bool,
    },
    Like(GraphExpression),
    Named {
        typed: bool,
        name: GraphTypeReference,
    },
    Nested {
        typed: bool,
        property_graph: bool,
        body: GraphTypeBody,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum CreateGraphTypeSource {
    CopyOf(GraphTypeReference),
    CopyOfExternal(String),
    Like(GraphExpression),
    Nested(GraphTypeBody),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StartTransactionStatement {
    pub access_mode: Option<TransactionAccessMode>,
    pub isolation_level: Option<IsolationLevel>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransactionAccessMode {
    ReadOnly,
    ReadWrite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IsolationLevel {
    Serializable,
    RepeatableRead,
    ReadCommitted,
    ReadUncommitted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitStatement;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RollbackStatement;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphName {
    pub absolute: bool,
    pub current_schema: bool,
    pub home_schema: bool,
    pub parent_levels: usize,
    pub parts: Vec<Identifier>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum GraphExpression {
    Variable(Box<Expr>),
    Name(GraphName),
    Parameter(String),
    CurrentGraph,
    CurrentPropertyGraph,
    HomeGraph,
    HomePropertyGraph,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BindingTableName {
    pub absolute: bool,
    pub current_schema: bool,
    pub home_schema: bool,
    pub parent_levels: usize,
    pub parts: Vec<Identifier>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum BindingTableExpression {
    NestedQuery(Box<QueryStatement>),
    Variable(Box<Expr>),
    Name(BindingTableName),
    Parameter(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphTypeName {
    pub absolute: bool,
    pub current_schema: bool,
    pub home_schema: bool,
    pub parent_levels: usize,
    pub parts: Vec<Identifier>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphTypeReference {
    Name(GraphTypeName),
    Parameter(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaName {
    pub absolute: bool,
    pub current_schema: bool,
    pub home_schema: bool,
    pub parent_levels: usize,
    pub parts: Vec<Identifier>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SchemaReference {
    Name(SchemaName),
    Absolute(SchemaName),
    Root,
    Parent { levels: usize, name: SchemaName },
    Parameter(String),
    CurrentSchema,
    HomeSchema,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcedureName {
    pub absolute: bool,
    pub current_schema: bool,
    pub home_schema: bool,
    pub parent_levels: usize,
    pub parts: Vec<Identifier>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PathPattern {
    pub variable: Option<Identifier>,
    pub prefix: Option<PathPatternPrefix>,
    pub parenthesized: Option<ParenthesizedPathPatternExpression>,
    pub start: NodePattern,
    pub chains: Vec<PathPatternChain>,
    pub factors: Vec<PathPatternFactor>,
    pub alternation: Option<PathPatternAlternation>,
    pub alternatives: Vec<PathPatternTerm>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParenthesizedPathPatternExpression {
    pub variable: Option<Identifier>,
    pub prefix: Option<PathPatternPrefix>,
    pub pattern: Box<PathPattern>,
    pub where_clause: Option<Expr>,
    pub quantifier: Option<PathPatternQuantifier>,
    pub questioned: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PathPatternTerm {
    pub start: NodePattern,
    pub chains: Vec<PathPatternChain>,
    pub factors: Vec<PathPatternFactor>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PathPatternFactor {
    Node(NodePattern),
    Relationship(RelationshipPattern),
    Parenthesized(Box<ParenthesizedPathPatternExpression>),
    Alternation {
        alternation: PathPatternAlternation,
        alternatives: Vec<Vec<PathPatternFactor>>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathPatternAlternation {
    Union,
    Multiset,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathPatternPrefix {
    Mode {
        mode: PathMode,
        path_or_paths: Option<PathOrPaths>,
    },
    Search(PathSearchPrefix),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathMode {
    Walk,
    Trail,
    Simple,
    Acyclic,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathOrPaths {
    Path,
    Paths,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathSearchPrefix {
    All {
        mode: Option<PathMode>,
        path_or_paths: Option<PathOrPaths>,
    },
    Any {
        count: Option<UnsignedIntegerSpecification>,
        mode: Option<PathMode>,
        path_or_paths: Option<PathOrPaths>,
    },
    Shortest {
        mode: Option<PathMode>,
        path_or_paths: Option<PathOrPaths>,
    },
    AllShortest {
        mode: Option<PathMode>,
        path_or_paths: Option<PathOrPaths>,
    },
    AnyShortest {
        mode: Option<PathMode>,
        path_or_paths: Option<PathOrPaths>,
    },
    CountedShortest {
        count: UnsignedIntegerSpecification,
        mode: Option<PathMode>,
        path_or_paths: Option<PathOrPaths>,
    },
    CountedShortestGroup {
        count: Option<UnsignedIntegerSpecification>,
        mode: Option<PathMode>,
        path_or_paths: Option<PathOrPaths>,
        groups: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnsignedIntegerSpecification {
    Literal(u64),
    Parameter(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct PathPatternChain {
    pub relationship: RelationshipPattern,
    pub node: NodePattern,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodePattern {
    pub variable: Option<Identifier>,
    pub temporary: bool,
    pub labels: Vec<Identifier>,
    pub label_expression: Option<LabelExpression>,
    pub properties: Option<MapLiteral>,
    pub where_clause: Option<Expr>,
    pub quantifier: Option<PathPatternQuantifier>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RelationshipPattern {
    pub direction: Direction,
    pub variable: Option<Identifier>,
    pub temporary: bool,
    pub labels: Vec<Identifier>,
    pub label_expression: Option<LabelExpression>,
    pub properties: Option<MapLiteral>,
    pub where_clause: Option<Expr>,
    pub quantifier: Option<PathPatternQuantifier>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Undirected,
    LeftOrUndirected,
    UndirectedOrRight,
    LeftOrRight,
    Any,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LabelExpression {
    Label(Identifier),
    Wildcard,
    And(Box<LabelExpression>, Box<LabelExpression>),
    Or(Box<LabelExpression>, Box<LabelExpression>),
    Not(Box<LabelExpression>),
    Parenthesized(Box<LabelExpression>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathPatternQuantifier {
    ZeroOrMore,
    OneOrMore,
    Optional,
    Fixed(u64),
    Range { min: Option<u64>, max: Option<u64> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphTypeBody {
    pub elements: Vec<GraphTypeElement>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphTypeElement {
    Node(NodeTypeDefinition),
    Edge(EdgeTypeDefinition),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeTypeDefinition {
    pub name: Identifier,
    pub labels: Vec<Identifier>,
    pub properties: Vec<PropertyTypeDefinition>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EdgeTypeDefinition {
    pub name: Identifier,
    pub direction: Direction,
    pub labels: Vec<Identifier>,
    pub source: Option<Identifier>,
    pub destination: Option<Identifier>,
    pub properties: Vec<PropertyTypeDefinition>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PropertyTypeDefinition {
    pub name: Identifier,
    pub value_type: ValueType,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValueType {
    Named {
        name: Identifier,
        parameters: Vec<u64>,
    },
    CharacterString {
        kind: CharacterStringTypeKind,
        max_length: Option<u64>,
    },
    Boolean {
        kind: BooleanTypeKind,
    },
    Temporal {
        kind: TemporalTypeKind,
    },
    ByteString {
        kind: ByteStringTypeKind,
        min_length: Option<u64>,
        max_length: Option<u64>,
    },
    ExactNumeric {
        kind: ExactNumericTypeKind,
        signed: Option<bool>,
        precision: Option<u64>,
        scale: Option<u64>,
    },
    ApproximateNumeric {
        kind: ApproximateNumericTypeKind,
        precision: Option<u64>,
        scale: Option<u64>,
    },
    List(Box<ValueType>),
    ListWithLength {
        element: Box<ValueType>,
        max_length: u64,
    },
    Array(Box<ValueType>),
    ArrayWithLength {
        element: Box<ValueType>,
        max_length: u64,
    },
    Path,
    AnyRecord,
    Record(Vec<FieldType>),
    AnyDynamic,
    PropertyValue,
    DynamicUnion(Vec<ValueType>),
    GraphReference {
        property_graph: bool,
        body: Option<GraphTypeBody>,
    },
    NodeReference {
        definition: Option<NodeTypeDefinition>,
    },
    EdgeReference {
        definition: Option<EdgeTypeDefinition>,
    },
    BindingTable {
        binding: bool,
        fields: Vec<FieldType>,
    },
    NotNull(Box<ValueType>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CharacterStringTypeKind {
    String,
    Varchar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BooleanTypeKind {
    Bool,
    Boolean,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemporalTypeKind {
    ZonedDateTime,
    LocalDateTime,
    Date,
    ZonedTime,
    LocalTime,
    Duration,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ByteStringTypeKind {
    Bytes,
    Binary,
    Varbinary,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExactNumericTypeKind {
    Decimal,
    Integer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApproximateNumericTypeKind {
    Float,
    Real,
    Double,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldType {
    pub name: Identifier,
    pub value_type: ValueType,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Identifier(Identifier),
    Property {
        base: Box<Expr>,
        key: Identifier,
    },
    GraphReference {
        property_graph: bool,
        graph: GraphExpression,
    },
    BindingTableReference {
        binding: bool,
        table: BindingTableExpression,
    },
    Parameter(String),
    Literal(Literal),
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Binary {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
    },
    Function {
        name: Identifier,
        quantifier: Option<SetQuantifier>,
        args: Vec<Expr>,
    },
    SubstringByLength {
        side: SubstringSide,
        value: Box<Expr>,
        length: Box<Expr>,
    },
    Trim {
        specification: Option<TrimSpec>,
        trim_character: Option<Box<Expr>>,
        value: Box<Expr>,
    },
    MultiTrim {
        specification: TrimSpec,
        value: Box<Expr>,
        trim_characters: Option<Box<Expr>>,
    },
    TrimList {
        value: Box<Expr>,
        count: Box<Expr>,
    },
    Elements {
        path: Box<Expr>,
    },
    StringLength {
        unit: StringLengthUnit,
        value: Box<Expr>,
    },
    Fold {
        upper: bool,
        value: Box<Expr>,
    },
    Normalize {
        value: Box<Expr>,
        normal_form: Option<Identifier>,
    },
    CurrentDate,
    CurrentTime {
        precision: Option<u64>,
    },
    CurrentTimestamp {
        precision: Option<u64>,
    },
    CurrentUser,
    DateTimeFunction {
        function: DateTimeFunctionKind,
        value: Option<Box<Expr>>,
    },
    LocalTime {
        precision: Option<u64>,
    },
    LocalTimestamp {
        precision: Option<u64>,
    },
    AbsoluteValue {
        expr: Box<Expr>,
    },
    DurationFunction {
        value: Box<Expr>,
    },
    DurationBetween {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    NumericFunction {
        function: NumericFunctionKind,
        args: Vec<Expr>,
    },
    Cast {
        expr: Box<Expr>,
        value_type: ValueType,
    },
    ValueQuery {
        query: Box<QueryStatement>,
    },
    Let {
        items: Vec<LetItem>,
        value: Box<Expr>,
    },
    IsTyped {
        expr: Box<Expr>,
        negated: bool,
        value_type: ValueType,
    },
    IsNull {
        expr: Box<Expr>,
        negated: bool,
    },
    IsUnknown {
        expr: Box<Expr>,
        negated: bool,
    },
    IsTruth {
        expr: Box<Expr>,
        negated: bool,
        value: bool,
    },
    PropertyExists {
        variable: Identifier,
        property: Identifier,
    },
    ElementId {
        variable: Identifier,
    },
    Exists {
        patterns: Vec<PathPattern>,
    },
    ExistsQuery {
        query: Box<QueryStatement>,
    },
    ExistsMatch {
        matches: Vec<MatchClause>,
    },
    Same {
        variables: Vec<Identifier>,
    },
    AllDifferent {
        variables: Vec<Identifier>,
    },
    SourceDestination {
        node: Identifier,
        negated: bool,
        kind: SourceDestinationKind,
        edge: Identifier,
    },
    IsNormalized {
        expr: Box<Expr>,
        negated: bool,
        normal_form: Option<Identifier>,
    },
    IsDirected {
        variable: Identifier,
        negated: bool,
    },
    IsLabeled {
        variable: Identifier,
        negated: bool,
        label_expression: LabelExpression,
    },
    Case {
        operand: Option<Box<Expr>>,
        when_clauses: Vec<CaseWhenClause>,
        else_expr: Option<Box<Expr>>,
    },
    Coalesce(Vec<Expr>),
    NullIf {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    List(Vec<Expr>),
    TypedList {
        type_name: ListValueTypeName,
        values: Vec<Expr>,
    },
    Path(Vec<Expr>),
    Record {
        explicit: bool,
        fields: MapLiteral,
    },
    Map(MapLiteral),
    Wildcard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrimSpec {
    Leading,
    Trailing,
    Both,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringLengthUnit {
    Characters,
    Bytes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubstringSide {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericFunctionKind {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateTimeFunctionKind {
    Date,
    ZonedTime,
    ZonedDateTime,
    LocalTime,
    LocalDateTime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListValueTypeName {
    List,
    Array,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CaseWhenClause {
    pub condition: Expr,
    pub additional_operands: Vec<Expr>,
    pub result: Expr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceDestinationKind {
    Source,
    Destination,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MapLiteral {
    pub entries: Vec<(Identifier, Expr)>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Literal {
    Null,
    Boolean(bool),
    Unknown,
    Integer(i64),
    Decimal(f64),
    String(String),
    Bytes(Vec<u8>),
    Date(String),
    Time(String),
    DateTime(String),
    Duration(String),
    SqlInterval {
        value: String,
        qualifier: SqlIntervalQualifier,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SqlIntervalQualifier {
    pub start: Identifier,
    pub start_precision: Option<u64>,
    pub end: Option<Identifier>,
    pub end_precision: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnaryOp {
    Not,
    Pos,
    Neg,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinaryOp {
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

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Identifier {
    pub value: String,
}

impl Identifier {
    pub fn new(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
        }
    }
}
