use crate::{gql as ast, Error, Result, SessionState, Value};
use datafusion::{
    arrow::datatypes::DataType,
    common::{Column, DFSchema, ScalarValue},
    logical_expr::{expr::BinaryExpr, Expr, ExprSchemable, Operator},
};

pub(super) struct Binder<'a> {
    session: &'a SessionState,
    schema: &'a DFSchema,
}

impl<'a> Binder<'a> {
    pub fn new(session: &'a SessionState, schema: &'a DFSchema) -> Self {
        Self { session, schema }
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
                let mut value = match &parameter.value {
                    Value::Null => ScalarValue::Null,
                    Value::Boolean(v) => ScalarValue::Boolean(Some(*v)),
                    Value::Integer(v) => ScalarValue::Int64(Some(*v)),
                    Value::Float(v) => ScalarValue::Float64(Some(*v)),
                    Value::String(v) => ScalarValue::Utf8(Some(v.clone())),
                    Value::Bytes(v) => ScalarValue::Binary(Some(v.clone())),
                    _ => return Err(super::unsupported("non-scalar query parameter")),
                };
                if let Some(declared) = &parameter.declared_type {
                    let data_type = match declared.kind.as_str() {
                        "any" => value.data_type(),
                        "boolean" => DataType::Boolean,
                        "integer" => DataType::Int64,
                        "float" => DataType::Float64,
                        "string" => DataType::Utf8,
                        "bytes" => DataType::Binary,
                        _ => return Err(super::unsupported("declared query parameter type")),
                    };
                    value = value.cast_to(&data_type)?;
                }
                datafusion::logical_expr::lit(value)
            }
            E::Identifier(name) => {
                if !self.schema.has_column_with_unqualified_name(&name.value) {
                    return Err(Error::InvalidQuery(format!(
                        "unbound variable {}",
                        name.value
                    )));
                }
                // Do not parse dots or quotes in an already-decoded GQL identifier as SQL.
                Expr::Column(Column::new_unqualified(&name.value))
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
                let left = self.bind(left)?;
                let right = self.bind(right)?;
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
                let expr = self.bind(expr)?;
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
                datafusion::functions::core::expr_fn::nullif(left, right)
            }
            _ => return Err(super::unsupported("query expression")),
        })
    }

    fn compatible(&self, expressions: &[Expr]) -> Result<()> {
        let mut previous = DataType::Null;
        for expr in expressions {
            let next = expr.get_type(self.schema)?;
            if previous != DataType::Null
                && next != DataType::Null
                && previous != next
                && !(numeric(&previous) && numeric(&next))
            {
                return Err(Error::InvalidQuery(format!(
                    "incompatible value types {previous} and {next}"
                )));
            }
            if next != DataType::Null {
                previous = next;
            }
        }
        Ok(())
    }
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
