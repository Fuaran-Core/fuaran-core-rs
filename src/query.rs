//! The query declaration — the data-acquisition sibling of the capability: a
//! declared, registrable read, typed by the parameters it takes and the columns
//! it answers — with its codec, the admission gate the registry and the reader
//! share, the identity of one read and the grant bound to it, and a query
//! registry deciding reads by the capability registry's own decision
//! (DECISIONS.md D14).
//!
//! - [`Query`] and its parts ([`QueryParam`], [`ColumnPredicate`], [`SortKey`])
//!   are the reference's records as plain data.
//! - [`Query::admission_fault_at`] is THE gate: a repeated parameter, then the
//!   filter, then the order, each held to the result's columns;
//!   [`Query::admit`] answers the [`AdmittedQuery`] it admits.
//! - [`AdmittedQuery::decode`] and [`AdmittedQuery::encode`] read and write the
//!   canonical declaration, the filter and the order through the seams' one
//!   reader ([`crate::seam`]).
//! - [`AdmittedQuery::invocation_identity`] is the identity a [`Grant`] for a
//!   read is bound to, written as it is produced; [`Grant::covers_read`]
//!   compares a grant with it as it is written, building nothing.
//! - [`QueryRegistry`] resolves, validates ([`Query::validate_params`]) and then
//!   decides through the function the capability registry decides by, so one
//!   function decides at both registries (D14.6).
//!
//! Each is the twin of the reference's `Fuaran.Core.Query` (`QueryTypes`,
//! `Query`, `QueryCodec`, `QueryRegistry`), certified against `queryGrantLaws`
//! and `queryDeclarationLaws`.
//!
//! An [`AdmittedQuery`] is made only by the gate ([`Query::admit`]), so no
//! declaration the gate refuses is written, keyed or granted (DECISIONS.md D18):
//!
//! ```
//! use fuaran_core::query::{AdmittedQuery, Query};
//! fn admit(q: Query) -> Option<AdmittedQuery> {
//!     q.admit().ok()
//! }
//! ```
//!
//! and built any other way from outside the crate, it is refused at compile
//! time (its field is private):
//!
//! ```compile_fail,E0451
//! use fuaran_core::query::{AdmittedQuery, Query};
//! fn forge(q: Query) -> AdmittedQuery {
//!     AdmittedQuery { query: q }
//! }
//! ```

use crate::capability::{
    EffectClass, Grant, Measured, PolicyDecision, PolicyGate, Unmatched, decide_granted, effect_of,
    effect_to_json, in_address_order,
};
use crate::column::{self, Cell, Column, ColumnError, ColumnType, DataSource, Table};
use crate::decode::{self, Cursor, DecodeCode, DecodeError, quote_all};
use crate::field::{self, Field, Schema};
use crate::hash::{FOLD_SEP, FieldBody, write_canonical_field};
use crate::json::{JVal, JsonLimits, PathSeg, escape_string, ordinal_cmp, render_canonical};
use crate::number::write_finite_double;
use crate::prelude::*;
use crate::seam::{self, Reading, Slot};
use alloc::sync::Arc;
use core::fmt::{self, Write};

// ─── The declaration ─────────────────────────────────────────────────────────

/// A typed parameter a query takes at invocation: a bound cell must widen to its
/// type ([`ColumnType::widens`]) or be `Null`; a required one must be bound to a
/// value. Twin of `QueryParam`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryParam {
    /// The name an argument binds under; unique within a query.
    pub name: String,
    /// The declared scalar type.
    pub ty: ColumnType,
    /// Whether it must be bound to a non-`Null` cell before the resolver runs.
    pub required: bool,
}

/// One typed predicate over a column of a query's result, a member of the
/// conjunction its filter declares: a comparison with a present literal of
/// exactly its column's type, an ordinal substring test on a string column, or
/// a test for absence or presence. Twin of `ColumnPredicate`.
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnPredicate {
    /// The column's cell equals `value`.
    EqualTo { column: String, value: Cell },
    /// The column's cell orders after `value`.
    GreaterThan { column: String, value: Cell },
    /// The column's cell orders at or after `value`.
    AtLeast { column: String, value: Cell },
    /// The column's cell orders before `value`.
    LessThan { column: String, value: Cell },
    /// The column's cell orders at or before `value`.
    AtMost { column: String, value: Cell },
    /// The column's string cell contains `text`, ordinally and case-sensitively.
    Contains { column: String, text: String },
    /// The column's cell is `Null`.
    IsNull { column: String },
    /// The column's cell is present.
    IsNotNull { column: String },
}

impl ColumnPredicate {
    /// The predicate's wire tag (`QueryShape.tag`), one of [`seam::PREDICATE_TAGS`].
    pub fn tag(&self) -> &'static str {
        match self {
            ColumnPredicate::EqualTo { .. } => "equalTo",
            ColumnPredicate::GreaterThan { .. } => "greaterThan",
            ColumnPredicate::AtLeast { .. } => "atLeast",
            ColumnPredicate::LessThan { .. } => "lessThan",
            ColumnPredicate::AtMost { .. } => "atMost",
            ColumnPredicate::Contains { .. } => "contains",
            ColumnPredicate::IsNull { .. } => "isNull",
            ColumnPredicate::IsNotNull { .. } => "isNotNull",
        }
    }

    /// The column the predicate names (`QueryShape.column`).
    pub fn column(&self) -> &str {
        match self {
            ColumnPredicate::EqualTo { column, .. }
            | ColumnPredicate::GreaterThan { column, .. }
            | ColumnPredicate::AtLeast { column, .. }
            | ColumnPredicate::LessThan { column, .. }
            | ColumnPredicate::AtMost { column, .. }
            | ColumnPredicate::Contains { column, .. }
            | ColumnPredicate::IsNull { column }
            | ColumnPredicate::IsNotNull { column } => column,
        }
    }

    /// The literal a comparison carries; `None` for `contains` and the null tests
    /// (`QueryShape.literal`).
    pub fn literal(&self) -> Option<&Cell> {
        match self {
            ColumnPredicate::EqualTo { value, .. }
            | ColumnPredicate::GreaterThan { value, .. }
            | ColumnPredicate::AtLeast { value, .. }
            | ColumnPredicate::LessThan { value, .. }
            | ColumnPredicate::AtMost { value, .. } => Some(value),
            ColumnPredicate::Contains { .. }
            | ColumnPredicate::IsNull { .. }
            | ColumnPredicate::IsNotNull { .. } => None,
        }
    }

    /// The predicate's document (`QuerySeamCodec.encodeColumnPredicate`): its
    /// `$type`, its `column`, and a comparison's literal as its `type` (the cell's
    /// own) and `value` (`QueryWire.comparedMembers`), `contains` its `text`.
    pub fn to_json(&self) -> JVal {
        let mut fields = vec![
            ("$type".to_string(), JVal::Str(self.tag().to_string())),
            ("column".to_string(), JVal::Str(self.column().to_string())),
        ];
        match self {
            ColumnPredicate::Contains { text, .. } => {
                fields.push(("text".to_string(), JVal::Str(text.clone())));
            }
            _ => {
                if let Some(v) = self.literal() {
                    // `Null` has no type: admission refuses it, and the writer stays total.
                    let ty = v.type_of().unwrap_or(ColumnType::String);
                    fields.push(("type".to_string(), JVal::Str(ty.tag().to_string())));
                    fields.push(("value".to_string(), literal_json(ty, v)));
                }
            }
        }
        JVal::Obj(fields)
    }

    /// The predicate the seams' reader answered for one document: a total
    /// projection of its re-encoding (a comparison's `type` is the cell's, its
    /// `value` the cell as the column codec wrote it), so `None` only for a
    /// document that reader never answers.
    fn of_read(v: &JVal) -> Option<ColumnPredicate> {
        let text = |k: &str| match v.field(k) {
            Some(JVal::Str(s)) => Some(s.clone()),
            _ => None,
        };
        let column = text("column")?;
        let compared = || -> Option<Cell> {
            let ty = ColumnType::of_tag(&text("type")?)?;
            Some(match (ty, v.field("value")?) {
                (ColumnType::Int, JVal::Num(n)) => Cell::Int(*n as i32),
                (ColumnType::Float, JVal::Num(n)) => Cell::Float(*n),
                (ColumnType::Bool, JVal::Bool(b)) => Cell::Bool(*b),
                (ColumnType::String, JVal::Str(s)) => Cell::Str(s.clone()),
                (ColumnType::Date, JVal::Str(s)) => Cell::Date(s.clone()),
                (ColumnType::Timestamp(_), JVal::Str(s)) => Cell::Timestamp(s.clone()),
                (ColumnType::Decimal, JVal::Str(s)) => Cell::Decimal(s.clone()),
                _ => return None,
            })
        };
        Some(match text("$type")?.as_str() {
            "equalTo" => ColumnPredicate::EqualTo {
                column,
                value: compared()?,
            },
            "greaterThan" => ColumnPredicate::GreaterThan {
                column,
                value: compared()?,
            },
            "atLeast" => ColumnPredicate::AtLeast {
                column,
                value: compared()?,
            },
            "lessThan" => ColumnPredicate::LessThan {
                column,
                value: compared()?,
            },
            "atMost" => ColumnPredicate::AtMost {
                column,
                value: compared()?,
            },
            "contains" => ColumnPredicate::Contains {
                column,
                text: text("text")?,
            },
            "isNull" => ColumnPredicate::IsNull { column },
            "isNotNull" => ColumnPredicate::IsNotNull { column },
            _ => return None,
        })
    }
}

/// A literal's JSON value: the value the column codec writes for that cell in a
/// column of `ty`; a literal its type cannot hold is written total rather than
/// refused, a date or timestamp as its text, any other cell at its own type, and
/// the empty string past that. Twin of `QueryWire.literalJson`.
fn literal_json(ty: ColumnType, c: &Cell) -> JVal {
    match ty.write_cell("v", c) {
        Ok(v) => v,
        Err(_) => match c {
            Cell::Date(s) | Cell::Timestamp(s) => JVal::Str(s.clone()),
            other => other
                .type_of()
                .unwrap_or(ty)
                .write_cell("v", other)
                .unwrap_or_else(|_| JVal::Str(String::new())),
        },
    }
}

/// The direction of one key of a query's declared order. Twin of `SortDirection`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    /// Smallest first, a `Null` cell before every present one.
    Ascending,
    /// Largest first, a `Null` cell after every present one.
    Descending,
}

impl SortDirection {
    /// The direction's wire value.
    pub fn tag(self) -> &'static str {
        match self {
            SortDirection::Ascending => "ascending",
            SortDirection::Descending => "descending",
        }
    }
}

/// One key of a query's declared order: a column of the result and its
/// direction. Twin of `SortKey`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortKey {
    /// A column of the query's result; an order names each column once.
    pub column: String,
    /// Which end of the order comes first.
    pub direction: SortDirection,
}

impl SortKey {
    /// The key's document (`QuerySeamCodec.encodeSortKey`): `{column, direction}`.
    pub fn to_json(&self) -> JVal {
        JVal::Obj(vec![
            ("column".to_string(), JVal::Str(self.column.clone())),
            (
                "direction".to_string(),
                JVal::Str(self.direction.tag().to_string()),
            ),
        ])
    }

    /// The key the seams' reader answered: `None` only for a document it never
    /// answers.
    fn of_read(v: &JVal) -> Option<SortKey> {
        let (Some(JVal::Str(column)), Some(JVal::Str(direction))) =
            (v.field("column"), v.field("direction"))
        else {
            return None;
        };
        let direction = match direction.as_str() {
            "ascending" => SortDirection::Ascending,
            "descending" => SortDirection::Descending,
            _ => return None,
        };
        Some(SortKey {
            column: column.clone(),
            direction,
        })
    }
}

/// A query declaration: a named, registrable read. Pure data — it carries no
/// host code; the host's resolver performs the read (DECISIONS.md D14.2). Twin of
/// `Query`.
#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    /// The registry key, and the first field of every read's identity.
    pub id: String,
    /// The parameters an invocation may bind, in declaration order.
    pub params: Vec<QueryParam>,
    /// The columns of one result row.
    pub result_schema: Schema,
    /// Whether the read is captured (a non-empty determinism set) or re-evaluated.
    pub effect: EffectClass,
    /// Where the rows come from: an embedded table, or a name only the host resolves.
    pub source: DataSource,
    /// The read's time budget in milliseconds, for the resolver to honour.
    pub timeout_ms: Option<i32>,
    /// The rows per page the host should return.
    pub page_size: Option<i32>,
    /// The filter, the wire's `where`: a conjunction, empty for none.
    pub filter: Vec<ColumnPredicate>,
    /// The order, most significant key first; empty leaves it to the source.
    pub order_by: Vec<SortKey>,
}

// ─── The refusal ─────────────────────────────────────────────────────────────

/// Why an invocation, a registration or a declaration was refused: the whole of
/// the reference's closed union, so a host's resolver refuses in the seam's own
/// vocabulary (DECISIONS.md D14.2). Twin of `QueryError`.
#[derive(Debug, Clone, PartialEq)]
pub enum QueryError {
    /// The id is not registered; `known` lists every registered id, in id order.
    NoSuchQuery { id: String, known: Vec<String> },
    /// A registration under an id the registry already holds.
    DuplicateQuery { id: String },
    /// An argument names no declared parameter; `declared` lists them in order.
    UnknownParam { name: String, declared: Vec<String> },
    /// An argument's cell type does not widen to its parameter's type.
    ParamTypeMismatch {
        name: String,
        expected: ColumnType,
        got: ColumnType,
    },
    /// The required parameters the arguments leave out, in declaration order.
    RequiredParamsUnbound { names: Vec<String> },
    /// The resolver could not resolve the query's `Ref` source (the wire's `ref`).
    SourceNotResolved { reference: String },
    /// The resolver ran and failed; `recoverable` names what a retry may change.
    ExecutionFailed {
        detail: String,
        recoverable: Vec<String>,
    },
    /// The resolver ran out of time.
    Timeout,
    /// The required parameters bound only to `Null`.
    RequiredParamsNull { names: Vec<String> },
    /// An invocation binds this parameter twice, or a declaration names it twice.
    DuplicateParam { name: String },
    /// A registry gate denied the read: the deciding gate, the denial's message
    /// and the alternatives it names, shared with the gate (DECISIONS.md D12).
    QueryPolicyRefused {
        policy: Arc<str>,
        reason: Arc<str>,
        allowed: Arc<[Arc<str>]>,
    },
    /// A registry gate requires an approval first, and no grant presented was
    /// for exactly this read.
    QueryApprovalRequired { policy: Arc<str> },
    /// The arguments could not be read at all.
    UnreadableArgs { error: DecodeError },
    /// A filter predicate or an order key names a column the result does not
    /// declare; `declared` lists the result's columns.
    UnknownColumn { name: String, declared: Vec<String> },
    /// A filter predicate compares its column with a literal of another type.
    PredicateTypeMismatch {
        column: String,
        expected: ColumnType,
        got: ColumnType,
    },
    /// A filter predicate (by its wire tag) does not apply to its column's type.
    PredicateNotApplicable {
        predicate: String,
        column: String,
        column_type: ColumnType,
    },
    /// A filter predicate's literal is not a value its column can carry.
    IllFormedLiteral { column: String, reason: String },
    /// An order names this column more than once.
    DuplicateSortColumn { column: String },
    /// The resolver cannot apply this filter predicate.
    PredicateNotHonoured { predicate: ColumnPredicate },
    /// The resolver cannot order the rows by this column.
    OrderNotHonoured { column: String },
}

/// How to write a value of a type a JSON scalar does not carry as itself.
fn how_to_write(t: ColumnType) -> String {
    match t {
        ColumnType::Decimal => " Write a decimal as a JSON string of decimal text, such as \"12.50\": an optional '-', digits, and an optional '.' followed by digits; never as a JSON number.".to_string(),
        ColumnType::Date => " Write a date as a JSON string, YYYY-MM-DD.".to_string(),
        ColumnType::Timestamp(column::TimeUnit::Seconds) => {
            " Write a timestamp as a JSON string, YYYY-MM-DDThh:mm:ssZ.".to_string()
        }
        ColumnType::Timestamp(u) => format!(
            " Write a timestamp as a JSON string, YYYY-MM-DDThh:mm:ssZ, or YYYY-MM-DDThh:mm:ss.FZ with up to {} fraction digits and no trailing zero.",
            u.digits()
        ),
        _ => String::new(),
    }
}

/// A decode path as the reference renders it (`DecodePath.render`): `$`, then
/// `[i]` for an item and `["k"]` for a member, the key JSON-escaped.
fn render_path(path: &[PathSeg]) -> String {
    let mut out = String::from("$");
    for seg in path {
        match seg {
            PathSeg::Index(i) => out.push_str(&format!("[{i}]")),
            PathSeg::Key(k) => out.push_str(&format!("[{}]", escape_string(k))),
        }
    }
    out
}

impl QueryError {
    /// One sentence a model can act on, naming the failure and what would be
    /// accepted. Twin of `QueryError.describe`.
    pub fn describe(&self) -> String {
        match self {
            QueryError::NoSuchQuery { id, known } if known.is_empty() => format!(
                "Refused: there is no query '{id}' you may run. There are no queries you may run."
            ),
            QueryError::NoSuchQuery { id, known } => format!(
                "Refused: there is no query '{id}' you may run. The queries you may run are {}.",
                quote_all(known)
            ),
            QueryError::DuplicateQuery { id } => {
                format!("Refused: the query '{id}' is registered twice.")
            }
            QueryError::UnknownParam { name, declared } if declared.is_empty() => format!(
                "Refused: '{name}' is not a parameter of this query. It takes no parameters."
            ),
            QueryError::UnknownParam { name, declared } => format!(
                "Refused: '{name}' is not a parameter of this query. Its parameters are {}.",
                quote_all(declared)
            ),
            QueryError::ParamTypeMismatch {
                name,
                expected,
                got,
            } => format!(
                "Refused: parameter '{name}' must be {}; you sent {}.{}",
                expected.tag(),
                got.tag(),
                how_to_write(*expected)
            ),
            QueryError::RequiredParamsUnbound { names } => format!(
                "Refused: required parameters missing: {}.",
                quote_all(names)
            ),
            QueryError::SourceNotResolved { reference } => {
                format!("Refused: the data source '{reference}' could not be resolved.")
            }
            QueryError::ExecutionFailed {
                detail,
                recoverable,
            } if recoverable.is_empty() => {
                format!("Refused: the query ran and failed: {detail}.")
            }
            QueryError::ExecutionFailed {
                detail,
                recoverable,
            } => format!(
                "Refused: the query ran and failed: {detail}. You may retry with {}.",
                quote_all(recoverable)
            ),
            QueryError::Timeout => "Refused: the query timed out.".to_string(),
            QueryError::RequiredParamsNull { names } => format!(
                "Refused: required parameters bound to no value: {}.",
                quote_all(names)
            ),
            QueryError::DuplicateParam { name } => format!(
                "Refused: parameter '{name}' is given more than once. Give each parameter once."
            ),
            QueryError::QueryPolicyRefused {
                policy,
                reason,
                allowed,
            } if allowed.is_empty() => {
                format!("Refused: the policy '{policy}' does not allow this: {reason}.")
            }
            QueryError::QueryPolicyRefused {
                policy,
                reason,
                allowed,
            } => format!(
                "Refused: the policy '{policy}' does not allow this: {reason}. What it allows instead: {}.",
                quote_all(allowed)
            ),
            QueryError::QueryApprovalRequired { policy } => format!(
                "Refused: the policy '{policy}' requires an approval before this query runs."
            ),
            QueryError::UnreadableArgs { error } => format!(
                "Refused: the arguments could not be read at {}: {}. Send one JSON object keyed by parameter name.",
                render_path(&error.path),
                error.message
            ),
            QueryError::UnknownColumn { name, declared } if declared.is_empty() => format!(
                "Refused: '{name}' is not a column of this query's result. It returns no columns."
            ),
            QueryError::UnknownColumn { name, declared } => format!(
                "Refused: '{name}' is not a column of this query's result. Its columns are {}.",
                quote_all(declared)
            ),
            QueryError::PredicateTypeMismatch {
                column,
                expected,
                got,
            } => format!(
                "Refused: the filter on column '{column}' compares with a {} value; the column is {}, and a filter compares with a value of the column's own type.{}",
                got.tag(),
                expected.tag(),
                how_to_write(*expected)
            ),
            QueryError::PredicateNotApplicable {
                predicate,
                column,
                column_type,
            } => format!(
                "Refused: the filter '{predicate}' does not apply to column '{column}', which is {}. It applies to string columns only.",
                column_type.tag()
            ),
            QueryError::IllFormedLiteral { column, reason } => format!(
                "Refused: the filter on column '{column}' compares with a value the column cannot carry: {reason}."
            ),
            QueryError::DuplicateSortColumn { column } => format!(
                "Refused: the order names column '{column}' more than once. Name each column once."
            ),
            QueryError::PredicateNotHonoured { predicate } => format!(
                "Refused: the data source cannot apply the filter '{}' on column '{}'.",
                predicate.tag(),
                predicate.column()
            ),
            QueryError::OrderNotHonoured { column } => {
                format!("Refused: the data source cannot order the rows by column '{column}'.")
            }
        }
    }
}

// ─── The admission gate ──────────────────────────────────────────────────────

/// Why a literal of its column's type is not one its column carries, from the
/// column layer's refusal (`QueryShape.literalReason`).
fn literal_reason(e: &ColumnError) -> String {
    match e {
        ColumnError::NonFiniteFloat { value, .. } => {
            format!("the float {value} is not finite, and the wire has no non-finite float")
        }
        ColumnError::MalformedShape(detail) => detail.clone(),
        _ => "it is not a value its column carries".to_string(),
    }
}

/// The refusal of a literal of its column's type that its column cannot carry:
/// `Column::of_cells`'s, else `Table::validate`'s (`QueryShape.literalFault`).
fn literal_fault(c: &str, ty: ColumnType, v: &Cell) -> Option<QueryError> {
    let refused = |e: ColumnError| {
        Some(QueryError::IllFormedLiteral {
            column: c.to_string(),
            reason: literal_reason(&e),
        })
    };
    match Column::of_cells(c, ty, core::slice::from_ref(v)) {
        Err(e) => refused(e),
        Ok(col) => {
            let table = Table {
                schema: vec![Field::create(c, col.column_type())],
                columns: vec![col],
            };
            match table.validate() {
                Ok(()) => None,
                Err(e) => refused(e),
            }
        }
    }
}

/// The result's column names, in schema order: what an `UnknownColumn` names.
fn columns_of(schema: &[Field]) -> Vec<String> {
    schema.iter().map(|f| f.name().to_string()).collect()
}

impl Query {
    /// The refusal one filter predicate earns against the result's columns:
    /// an undeclared column, `contains` on a column that is not a string, a
    /// `Null` literal, a literal of another type (a timestamp literal fits any
    /// timestamp column at least as fine as its own unit), then one its column
    /// cannot carry. Twin of `QueryShape.whereFault`'s per-predicate check.
    fn predicate_fault(&self, p: &ColumnPredicate) -> Option<QueryError> {
        let c = p.column();
        let Some(f) = self.result_schema.iter().find(|f| f.name() == c) else {
            return Some(QueryError::UnknownColumn {
                name: c.to_string(),
                declared: columns_of(&self.result_schema),
            });
        };
        let ty = f.column_type();
        if matches!(p, ColumnPredicate::Contains { .. }) && ty != ColumnType::String {
            return Some(QueryError::PredicateNotApplicable {
                predicate: p.tag().to_string(),
                column: c.to_string(),
                column_type: ty,
            });
        }
        match p.literal() {
            None => None,
            Some(Cell::Null) => Some(QueryError::IllFormedLiteral {
                column: c.to_string(),
                reason: "a null literal compares with nothing; test for absence with isNull"
                    .to_string(),
            }),
            Some(v) => match (v.type_of(), ty) {
                (Some(ColumnType::Timestamp(got)), ColumnType::Timestamp(unit))
                    if got.widens(unit) =>
                {
                    literal_fault(c, ty, v)
                }
                (Some(got), _) if got != ty => Some(QueryError::PredicateTypeMismatch {
                    column: c.to_string(),
                    expected: ty,
                    got,
                }),
                _ => literal_fault(c, ty, v),
            },
        }
    }

    /// The registration refusal the declaration earns, with the path to the
    /// member at fault, or `None`: a parameter named twice (`DuplicateParam`, the
    /// first repeat, at `params`); then each filter predicate in order (at
    /// `where.i`); then each order key (at `orderBy.i`): an undeclared column,
    /// then a column named before (`DuplicateSortColumn`, at the second). THE
    /// gate: [`QueryRegistry::register`] and [`AdmittedQuery::decode`] both run it,
    /// through [`Query::admit`] (DECISIONS.md D14.4, D18.2). Twin of
    /// `QueryRegistry.admissionFaultAt`.
    pub fn admission_fault_at(&self) -> Option<(Vec<PathSeg>, QueryError)> {
        let key = |k: &str| PathSeg::Key(k.to_string());
        if let Some(dup) = self
            .params
            .iter()
            .enumerate()
            .find(|(i, p)| self.params[..*i].iter().any(|q| q.name == p.name))
        {
            return Some((
                vec![key("params")],
                QueryError::DuplicateParam {
                    name: dup.1.name.clone(),
                },
            ));
        }
        for (i, p) in self.filter.iter().enumerate() {
            if let Some(e) = self.predicate_fault(p) {
                return Some((vec![key("where"), PathSeg::Index(i)], e));
            }
        }
        for (i, k) in self.order_by.iter().enumerate() {
            let fault = if !self.result_schema.iter().any(|f| f.name() == k.column) {
                Some(QueryError::UnknownColumn {
                    name: k.column.clone(),
                    declared: columns_of(&self.result_schema),
                })
            } else if self.order_by[..i].iter().any(|x| x.column == k.column) {
                Some(QueryError::DuplicateSortColumn {
                    column: k.column.clone(),
                })
            } else {
                None
            };
            if let Some(e) = fault {
                return Some((vec![key("orderBy"), PathSeg::Index(i)], e));
            }
        }
        None
    }

    /// [`Query::admission_fault_at`] without the path. Twin of
    /// `QueryRegistry.admissionFault`.
    pub fn admission_fault(&self) -> Option<QueryError> {
        self.admission_fault_at().map(|(_, e)| e)
    }
}

// ─── The admitted declaration ────────────────────────────────────────────────

pub use admitted::AdmittedQuery;

/// The admitted declaration in a module of its own, so its field is nameable by
/// the gate and the two accessors alone: nothing else in the crate, and nothing
/// outside it, can build one but through [`Query::admit`] (DECISIONS.md D18.2).
mod admitted {
    use super::{PathSeg, Query, QueryError};
    use crate::prelude::*;

    /// A declaration the admission gate admitted: the only declaration the
    /// writer ([`AdmittedQuery::encode`]), the read's identity
    /// ([`AdmittedQuery::invocation_identity`]) and its grant take, so no
    /// declaration the gate refuses is written, keyed or granted. Made only by
    /// [`Query::admit`]; answered by the reader ([`AdmittedQuery::decode`]) and
    /// by a registry's [`try_find`](super::QueryRegistry::try_find) and
    /// [`enumerate`](super::QueryRegistry::enumerate). Two admissions of one
    /// declaration are equal. The gate does not read the declaration's source.
    /// Twin of `AdmittedQuery`.
    #[derive(Debug, Clone, PartialEq)]
    pub struct AdmittedQuery {
        query: Query,
    }

    impl AdmittedQuery {
        /// The declaration the gate admitted. Twin of `AdmittedQuery.Declaration`.
        pub fn declaration(&self) -> &Query {
            &self.query
        }

        /// The declaration the gate admitted, by value.
        pub fn into_declaration(self) -> Query {
            self.query
        }
    }

    impl Query {
        /// THE gate, answering the admitted declaration, or the refusal and the
        /// path to the member at fault ([`Query::admission_fault_at`]). The one
        /// place an [`AdmittedQuery`] is made. Twin of `QueryRegistry.admitAt`.
        pub(super) fn admit_at(self) -> Result<AdmittedQuery, (Vec<PathSeg>, QueryError)> {
            match self.admission_fault_at() {
                None => Ok(AdmittedQuery { query: self }),
                Some(fault) => Err(fault),
            }
        }

        /// Admit the declaration: the admitted declaration, or the refusal
        /// [`Query::admission_fault`] names, the one
        /// [`register`](super::QueryRegistry::register) answers for an id it
        /// does not hold. It reads the declaration and nothing else, so a host
        /// runs it before it writes or keys anything. Twin of
        /// `QueryRegistry.admit`.
        pub fn admit(self) -> Result<AdmittedQuery, QueryError> {
            self.admit_at().map_err(|(_, e)| e)
        }
    }
}

// ─── The codec ───────────────────────────────────────────────────────────────

/// The seams' reader, read leniently (an unknown member is read past).
const LENIENT: Reading = Reading {
    strict: false,
    param: Slot::Param,
};

fn param_of(c: &Cursor<'_>) -> Result<QueryParam, DecodeError> {
    let name = c.field("name")?.string()?.to_string();
    let ty = seam::column_type_of(&c.field("type")?)?;
    let required = c.field("required")?.boolean()?;
    Ok(QueryParam { name, ty, required })
}

fn predicate_of(c: &Cursor<'_>) -> Result<ColumnPredicate, DecodeError> {
    let read = seam::read_at(&Slot::Union(&seam::COLUMN_PREDICATE), c, &LENIENT)?;
    ColumnPredicate::of_read(&read).ok_or_else(|| {
        c.fail(
            DecodeCode::SchemaFault,
            "the seams' reader answered a predicate the query module does not hold",
        )
    })
}

fn sort_key_of(c: &Cursor<'_>) -> Result<SortKey, DecodeError> {
    let read = seam::read_at(&Slot::Record(&seam::SORT_KEY), c, &LENIENT)?;
    SortKey::of_read(&read).ok_or_else(|| {
        c.fail(
            DecodeCode::SchemaFault,
            "the seams' reader answered an order key the query module does not hold",
        )
    })
}

/// The items of an optional array member, each read by `item`; absent is empty.
fn optional_list<T>(
    c: &Cursor<'_>,
    name: &str,
    item: impl Fn(&Cursor<'_>) -> Result<T, DecodeError>,
) -> Result<Vec<T>, DecodeError> {
    match c.optional(name)? {
        None => Ok(Vec::new()),
        Some(at) => at.items()?.iter().map(item).collect(),
    }
}

fn optional_int(c: &Cursor<'_>, name: &str) -> Result<Option<i32>, DecodeError> {
    c.optional(name)?.map(|at| at.int()).transpose()
}

impl Query {
    fn filter_json(&self) -> JVal {
        JVal::Arr(self.filter.iter().map(ColumnPredicate::to_json).collect())
    }

    fn order_json(&self) -> JVal {
        JVal::Arr(self.order_by.iter().map(SortKey::to_json).collect())
    }
}

impl AdmittedQuery {
    /// The declaration's document (`QueryCodec.queryJson`): `timeoutMs`,
    /// `pageSize`, `where` and `orderBy` omitted when absent or empty. It takes
    /// only an admitted declaration, so it never writes one the gate refuses
    /// (DECISIONS.md D18.3).
    pub fn encode_json(&self) -> JVal {
        let q = self.declaration();
        let mut fields = vec![
            ("$type".to_string(), JVal::Str("query".to_string())),
            ("id".to_string(), JVal::Str(q.id.clone())),
            (
                "params".to_string(),
                JVal::Arr(
                    q.params
                        .iter()
                        .map(|p| {
                            JVal::Obj(vec![
                                ("name".to_string(), JVal::Str(p.name.clone())),
                                ("type".to_string(), JVal::Str(p.ty.tag().to_string())),
                                ("required".to_string(), JVal::Bool(p.required)),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "resultSchema".to_string(),
                field::schema_json(&q.result_schema),
            ),
            ("effect".to_string(), effect_to_json(&q.effect)),
            ("source".to_string(), column::encode_json(&q.source)),
        ];
        let int = |n: i32| JVal::Num(f64::from(n));
        if let Some(t) = q.timeout_ms {
            fields.push(("timeoutMs".to_string(), int(t)));
        }
        if let Some(p) = q.page_size {
            fields.push(("pageSize".to_string(), int(p)));
        }
        if !q.filter.is_empty() {
            fields.push(("where".to_string(), q.filter_json()));
        }
        if !q.order_by.is_empty() {
            fields.push(("orderBy".to_string(), q.order_json()));
        }
        JVal::Obj(fields)
    }

    /// The declaration as canonical JSON text: the same declaration always
    /// writes the same bytes. Twin of `QueryCodec.encode`.
    pub fn encode(&self) -> String {
        render_canonical(&self.encode_json())
    }

    /// Read a declaration from its text (strict JSON, then
    /// [`AdmittedQuery::decode_value`]). Twin of `QueryCodec.decodeDetailedWith
    /// ReadPolicy.Lenient`.
    pub fn decode(text: &str) -> Result<AdmittedQuery, DecodeError> {
        let v = decode::parse(text, &JsonLimits::STRICT)?;
        AdmittedQuery::decode_value(&Cursor::root(&v))
    }

    /// Read a declaration leniently (an unknown member is read past), in the
    /// reference's order: `$type` (`query`), `id`, `params`, `resultSchema` (a
    /// column the column codec refuses is `OutOfRange` at its index), `effect`,
    /// `timeoutMs`, `pageSize`, `where` and `orderBy` (each item through the
    /// seams' reader), `source` (any JSON, then the column codec: `OutOfRange` at
    /// `source`), and last the admission gate, whose refusal is `OutOfRange` at
    /// the member at fault with the registry's sentence (DECISIONS.md D14.3): so
    /// it answers only a declaration the gate admitted (D18.3). Twin of
    /// `QueryCodec.decodeJson`.
    pub fn decode_value(c: &Cursor<'_>) -> Result<AdmittedQuery, DecodeError> {
        c.tag("$type", &["query"])?;
        let id = c.field("id")?.string()?.to_string();
        let params = c
            .field("params")?
            .items()?
            .iter()
            .map(param_of)
            .collect::<Result<Vec<_>, _>>()?;
        let result_schema = c
            .field("resultSchema")?
            .items()?
            .iter()
            .map(|f| {
                field::decode_field(f.value()).map_err(|e| {
                    f.fail(
                        DecodeCode::OutOfRange,
                        format!(
                            "not a result column the column codec reads: {}",
                            e.message()
                        ),
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let effect = effect_of(&c.field("effect")?)?;
        let timeout_ms = optional_int(c, "timeoutMs")?;
        let page_size = optional_int(c, "pageSize")?;
        let filter = optional_list(c, "where", predicate_of)?;
        let order_by = optional_list(c, "orderBy", sort_key_of)?;
        let source_at = c.field("source")?;
        let source = column::decode_value(source_at.value()).map_err(|e| {
            source_at.fail(
                DecodeCode::OutOfRange,
                format!(
                    "source: not a data source the column codec reads: {}",
                    e.message()
                ),
            )
        })?;
        let q = Query {
            id,
            params,
            result_schema,
            effect,
            source,
            timeout_ms,
            page_size,
            filter,
            order_by,
        };
        q.admit_at().map_err(|(at, e)| {
            let mut path = c.path().to_vec();
            path.extend(at);
            DecodeError {
                code: DecodeCode::OutOfRange,
                path,
                message: format!("a declaration the registry admits: {}", e.describe()),
            }
        })
    }
}

// ─── The read's identity and its grant ───────────────────────────────────────

/// The canonical text of a declaration's filter and order, each `None` when it
/// declares none: what the identity writes after the id (D14.5). Rendered once
/// per identity here, and once per query by a registry that holds it.
#[derive(Clone)]
struct Shape {
    filter: Option<String>,
    order: Option<String>,
}

impl Shape {
    fn of(q: &Query) -> Shape {
        Shape {
            filter: (!q.filter.is_empty()).then(|| render_canonical(&q.filter_json())),
            order: (!q.order_by.is_empty()).then(|| render_canonical(&q.order_json())),
        }
    }

    /// The identity of a read of `q` with `args`, written through `out` as it is
    /// produced: the id; the shape's fields (`""`, `w`, the filter's text; `""`,
    /// `o`, the order's); then each binding in name order, its name, its cell's
    /// tag and its payload. Builds nothing.
    fn write_identity<W: Write + ?Sized>(
        &self,
        q: &Query,
        args: &[(String, Cell)],
        out: &mut W,
    ) -> fmt::Result {
        write_canonical_field(&q.id, out)?;
        for (tag, text) in [("w", &self.filter), ("o", &self.order)] {
            if let Some(text) = text {
                write_canonical_field("", out)?;
                write_canonical_field(tag, out)?;
                write_canonical_field(text, out)?;
            }
        }
        in_address_order(args, |name, cell| {
            write_canonical_field(name, out)?;
            write_canonical_field(cell_tag(cell), out)?;
            write_payload(cell, &mut FieldBody(&mut *out))?;
            out.write_char(FOLD_SEP)
        })
    }

    /// Is `grant` for exactly the read of `q` with `args`: the identity compared
    /// with the grant's bytes as it is written, stopping at the first that differs.
    fn covers(&self, grant: &Grant, q: &Query, args: &[(String, Cell)]) -> bool {
        let mut rest = Unmatched(grant.invocation().as_bytes());
        self.write_identity(q, args, &mut rest).is_ok() && rest.0.is_empty()
    }
}

/// A cell's tag in the identity (`Query.cellFields`): `m` is a decimal's, as `d`
/// is a date's.
fn cell_tag(c: &Cell) -> &'static str {
    match c {
        Cell::Int(_) => "i",
        Cell::Float(_) => "f",
        Cell::Bool(_) => "b",
        Cell::Str(_) => "s",
        Cell::Date(_) => "d",
        Cell::Timestamp(_) => "t",
        Cell::Decimal(_) => "m",
        Cell::Null => "n",
    }
}

/// A cell's payload in the identity (`Query.cellFields`): an integer's decimal
/// digits; a float as `FloatLayout.canonical` writes it (the canonical number
/// form, a negative zero `0`, a non-finite float its quoted token); `1` or `0`;
/// a string, date, timestamp or decimal as it stands; empty for a null.
fn write_payload<W: Write + ?Sized>(c: &Cell, out: &mut W) -> fmt::Result {
    match c {
        Cell::Int(i) => write!(out, "{i}"),
        Cell::Float(f) if f.is_nan() => out.write_str("\"NaN\""),
        Cell::Float(f) if *f == f64::INFINITY => out.write_str("\"Infinity\""),
        Cell::Float(f) if *f == f64::NEG_INFINITY => out.write_str("\"-Infinity\""),
        Cell::Float(f) => write_finite_double(*f, out),
        Cell::Bool(b) => out.write_str(if *b { "1" } else { "0" }),
        Cell::Str(s) | Cell::Date(s) | Cell::Timestamp(s) | Cell::Decimal(s) => out.write_str(s),
        Cell::Null => Ok(()),
    }
}

impl AdmittedQuery {
    /// The IDENTITY of one read, what a [`Grant`] for a read is bound to: the
    /// query id, then for a declared filter the fields `""`, `w` and its
    /// canonical text, for a declared order `""`, `o` and its canonical text,
    /// then each binding in name order (ordinal; equal names in list order) as
    /// its name, its cell's tag and its payload, as one canonical pre-image with
    /// NO hash taken. A cell is identified by its tag, so an integer and the
    /// float of the same value are two reads; the page token is not in it, so a
    /// grant for a read covers every page (DECISIONS.md D14.5). Twin of
    /// `Query.invocationPreimage`, which takes an admitted declaration, as this
    /// does (D18.3).
    pub fn invocation_identity(&self, args: &[(String, Cell)]) -> String {
        let q = self.declaration();
        let shape = Shape::of(q);
        let mut measured = Measured(0);
        shape
            .write_identity(q, args, &mut measured)
            .expect("measuring accepts every write");
        let mut identity = String::with_capacity(measured.0);
        shape
            .write_identity(q, args, &mut identity)
            .expect("a String accepts every write");
        identity
    }

    /// [`AdmittedQuery::invocation_identity`] written through `out` as it is
    /// produced. It renders the declaration's filter and order text, the one
    /// thing it builds; a registry renders that once, when it registers the
    /// query, and its decision builds nothing (D14.6). `out`'s refusal stops the
    /// write.
    pub fn write_invocation_identity<W: Write + ?Sized>(
        &self,
        args: &[(String, Cell)],
        out: &mut W,
    ) -> fmt::Result {
        let q = self.declaration();
        Shape::of(q).write_identity(q, args, out)
    }

    /// The grant for exactly this read: what a host mints when an approval is
    /// given for a previewed read. Twin of `Query.grant`.
    pub fn grant(&self, args: &[(String, Cell)]) -> Grant {
        Grant::of_invocation(self.invocation_identity(args))
    }
}

impl Grant {
    /// Is the grant for exactly the read of `q` with `args`: what
    /// `covers(&q.invocation_identity(args))` answers, with the identity compared
    /// with the grant's bytes as it is written, stopping at the first byte that
    /// differs (DECISIONS.md D14.5). The page a read is dispatched for is not in
    /// the identity, so a grant covers every page of the read.
    pub fn covers_read(&self, q: &AdmittedQuery, args: &[(String, Cell)]) -> bool {
        let q = q.declaration();
        Shape::of(q).covers(self, q, args)
    }
}

// ─── Validation ──────────────────────────────────────────────────────────────

impl Query {
    fn param(&self, name: &str) -> Option<&QueryParam> {
        self.params.iter().find(|p| p.name == name)
    }

    fn param_names(&self) -> Vec<String> {
        self.params.iter().map(|p| p.name.clone()).collect()
    }

    /// Hold `args` to the declared parameters before any read, the first refusal
    /// in this order: a name an earlier binding binds (`DuplicateParam`); an
    /// argument naming no parameter (`UnknownParam`) or whose cell does not widen
    /// to its parameter's type (`ParamTypeMismatch`; a `Null` binds any
    /// parameter), argument by argument; required parameters left out
    /// (`RequiredParamsUnbound`); required parameters bound only to `Null`
    /// (`RequiredParamsNull`). Builds a refusal only when it answers one. Twin of
    /// `Query.validateParams`.
    pub fn validate_params(&self, args: &[(String, Cell)]) -> Result<(), QueryError> {
        if let Some((name, _)) = args
            .iter()
            .enumerate()
            .find(|(i, (n, _))| args[..*i].iter().any(|(m, _)| m == n))
            .map(|(_, a)| a)
        {
            return Err(QueryError::DuplicateParam { name: name.clone() });
        }
        for (name, cell) in args {
            let Some(p) = self.param(name) else {
                return Err(QueryError::UnknownParam {
                    name: name.clone(),
                    declared: self.param_names(),
                });
            };
            if let Some(got) = cell.type_of() {
                if !got.widens(p.ty) {
                    return Err(QueryError::ParamTypeMismatch {
                        name: name.clone(),
                        expected: p.ty,
                        got,
                    });
                }
            }
        }
        let unbound = |p: &&QueryParam| p.required && !args.iter().any(|(n, _)| *n == p.name);
        if self.params.iter().any(|p| unbound(&p)) {
            return Err(QueryError::RequiredParamsUnbound {
                names: self
                    .params
                    .iter()
                    .filter(unbound)
                    .map(|p| p.name.clone())
                    .collect(),
            });
        }
        let null_bound = |p: &&QueryParam| {
            p.required
                && !args
                    .iter()
                    .any(|(n, c)| *n == p.name && !matches!(c, Cell::Null))
        };
        if self.params.iter().any(|p| null_bound(&p)) {
            return Err(QueryError::RequiredParamsNull {
                names: self
                    .params
                    .iter()
                    .filter(null_bound)
                    .map(|p| p.name.clone())
                    .collect(),
            });
        }
        Ok(())
    }
}

// ─── The registry ────────────────────────────────────────────────────────────

/// The query registry's gate: a [`PolicyGate`] over a query and its typed
/// arguments, deciding under a context `C` (DECISIONS.md D14.6).
pub type QueryGate<C = ()> = PolicyGate<C, Query, [(String, Cell)]>;

/// A typed query registry: the discovery surface an agent enumerates,
/// default-deny on dispatch, and the gates its reads are decided by under a
/// decision context `C` (`()` for a registry that needs none), through the
/// decision the capability registry takes. Opaque: [`QueryRegistry::register`]
/// and its admission gate are the only way a query gets in, and it holds each
/// as the [`AdmittedQuery`] the gate answered (D18.4). Each query's filter and
/// order text is rendered once, when it is registered, so deciding a read
/// builds nothing (D14.6). Twin of `QueryRegistry<'Ctx>` (its denial observers
/// are not carried: a refusal is the dispatch's `Err`).
pub struct QueryRegistry<C = ()> {
    /// Held in ordinal id order, the order `enumerate` answers in.
    entries: Vec<AdmittedQuery>,
    /// Each entry's rendered filter and order, at the entry's index.
    shapes: Vec<Shape>,
    /// In the order they run.
    gates: Vec<QueryGate<C>>,
}

impl<C> Clone for QueryRegistry<C> {
    fn clone(&self) -> Self {
        QueryRegistry {
            entries: self.entries.clone(),
            shapes: self.shapes.clone(),
            gates: self.gates.clone(),
        }
    }
}

impl<C> Default for QueryRegistry<C> {
    fn default() -> Self {
        QueryRegistry {
            entries: Vec::new(),
            shapes: Vec::new(),
            gates: Vec::new(),
        }
    }
}

impl<C> core::fmt::Debug for QueryRegistry<C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("QueryRegistry")
            .field("entries", &self.entries)
            .field("gates", &self.gates)
            .finish()
    }
}

/// Equal when both hold the same queries and the same gates in the same order
/// (each by name and by identity of its decision function).
impl<C> PartialEq for QueryRegistry<C> {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries && self.gates == other.gates
    }
}

impl QueryRegistry<()> {
    /// The registry with no queries and no gates, deciding under no context.
    /// Twin of `QueryRegistry.empty` at `unit`.
    pub fn new() -> QueryRegistry<()> {
        QueryRegistry::empty()
    }
}

impl<C> QueryRegistry<C> {
    /// The registry with no queries and no gates, deciding under contexts of
    /// type `C`. Twin of `QueryRegistry.empty`.
    pub fn empty() -> QueryRegistry<C> {
        QueryRegistry::default()
    }

    fn position(&self, id: &str) -> Result<usize, usize> {
        self.entries
            .binary_search_by(|q| ordinal_cmp(&q.declaration().id, id))
    }

    /// Register a query: a held id is `DuplicateQuery` (checked first), and the
    /// admission gate's refusal otherwise ([`Query::admit`]); the registry holds
    /// the admitted declaration the gate answered, and is unchanged on a
    /// refusal. Twin of `QueryRegistry.register`.
    pub fn register(&self, q: Query) -> Result<QueryRegistry<C>, QueryError> {
        match self.position(&q.id) {
            Ok(_) => Err(QueryError::DuplicateQuery { id: q.id }),
            Err(at) => {
                let q = q.admit()?;
                let mut next = self.clone();
                next.shapes.insert(at, Shape::of(q.declaration()));
                next.entries.insert(at, q);
                Ok(next)
            }
        }
    }

    /// The query registered under exactly `id` (ordinal), as the gate admitted
    /// it. Twin of `QueryRegistry.tryFind`.
    pub fn try_find(&self, id: &str) -> Option<&AdmittedQuery> {
        self.position(id).ok().map(|i| &self.entries[i])
    }

    /// Every query, in ordinal id order whatever the insertion order, as the
    /// gate admitted it. Twin of `QueryRegistry.enumerate`.
    pub fn enumerate(&self) -> &[AdmittedQuery] {
        &self.entries
    }

    /// The held ids, in id order — what a `NoSuchQuery` names.
    pub fn ids(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|q| q.declaration().id.clone())
            .collect()
    }

    /// Remove the query under `id`; an id not held is `NoSuchQuery` naming every
    /// held id. Twin of `QueryRegistry.unregister`.
    pub fn unregister(&self, id: &str) -> Result<QueryRegistry<C>, QueryError> {
        let i = self.held(id)?;
        let mut next = self.clone();
        next.entries.remove(i);
        next.shapes.remove(i);
        Ok(next)
    }

    /// The registry with `gate` added after its gates: the decision is the join
    /// over every gate, so the result refuses everything this one refused, and
    /// possibly more. Twin of `QueryRegistry.withGate`.
    pub fn with_gate(&self, gate: QueryGate<C>) -> QueryRegistry<C> {
        let mut next = self.clone();
        next.gates.push(gate);
        next
    }

    /// The gate names, in the order they run.
    pub fn gates(&self) -> Vec<&str> {
        self.gates.iter().map(PolicyGate::policy).collect()
    }

    /// The index of the query under `id`, or `NoSuchQuery` naming the held ids.
    fn held(&self, id: &str) -> Result<usize, QueryError> {
        self.position(id).map_err(|_| QueryError::NoSuchQuery {
            id: id.to_string(),
            known: self.ids(),
        })
    }

    /// Resolve `id` and validate `args` — the default-deny half of dispatch that
    /// runs before any gate and any resolver: an id not held is `NoSuchQuery`,
    /// and the arguments are held to [`Query::validate_params`].
    pub fn resolve(&self, id: &str, args: &[(String, Cell)]) -> Result<&Query, QueryError> {
        let q = self.entries[self.held(id)?].declaration();
        q.validate_params(args)?;
        Ok(q)
    }

    /// The ONE judgement the named read and every dispatch share: resolve,
    /// validate, then the decision the capability registry takes
    /// ([`decide_granted`](crate::capability)), the grant read as covering when
    /// it is for exactly this read, its identity written from the text rendered
    /// at registration. Builds nothing on an admitted read (D14.6). Twin of
    /// `KeyedRegistry.judge` over `RegistryPolicy.decideGranted`.
    fn judge(
        &self,
        ctx: &C,
        grant: Option<&Grant>,
        id: &str,
        args: &[(String, Cell)],
    ) -> Result<(&Query, Option<&QueryGate<C>>, PolicyDecision), QueryError> {
        let i = self.held(id)?;
        let (q, shape) = (self.entries[i].declaration(), &self.shapes[i]);
        q.validate_params(args)?;
        let (decider, decision) = decide_granted(&self.gates, ctx, q, args, || {
            grant.is_some_and(|g| shape.covers(g, q, args))
        });
        Ok((q, decider, decision))
    }

    /// The registry's NAMED decision for a read, without dispatching it: `Ok` of
    /// the deciding gate's name, borrowed from the registry (`""` when every gate
    /// allows), and the decision (a `NeedsApproval` the grant is for lifted to
    /// `Allow`, the name kept; no grant lifts a `Deny`), or, where no gate
    /// decided, the refusal a dispatch gives: `NoSuchQuery` naming the held ids,
    /// or the validation refusal. Advisory: [`QueryRegistry::dispatch`] judges
    /// again, through the same function, and is authoritative. Twin of
    /// `QueryRegistry.decideNamed`.
    pub fn decide_named(
        &self,
        ctx: &C,
        grant: Option<&Grant>,
        id: &str,
        args: &[(String, Cell)],
    ) -> Result<(&str, PolicyDecision), QueryError> {
        self.judge(ctx, grant, id, args)
            .map(|(_, gate, d)| (gate.map_or("", PolicyGate::policy), d))
    }

    /// Dispatch a read: resolve the id (default-deny), validate the arguments,
    /// decide under `ctx` (a `Deny` is `QueryPolicyRefused`, whatever is
    /// presented; a `NeedsApproval` is `QueryApprovalRequired` unless `grant` is
    /// for exactly this read), and only then hand the resolved query to
    /// `resolve`, the host's resolver, exactly once; its refusal is the
    /// dispatch's. A refused dispatch never runs it. Twin of
    /// `QueryRegistry.dispatch`.
    pub fn dispatch<T>(
        &self,
        ctx: &C,
        grant: Option<&Grant>,
        id: &str,
        args: &[(String, Cell)],
        resolve: impl FnOnce(&Query) -> Result<T, QueryError>,
    ) -> Result<T, QueryError> {
        let (q, gate, decision) = self.judge(ctx, grant, id, args)?;
        // Only a deciding gate refuses, so a refusal always has one to name.
        let policy = || gate.map_or_else(|| Arc::from(""), |g| Arc::clone(&g.policy));
        match decision {
            PolicyDecision::Allow => resolve(q),
            PolicyDecision::NeedsApproval => {
                Err(QueryError::QueryApprovalRequired { policy: policy() })
            }
            PolicyDecision::Deny {
                message,
                alternatives,
            } => Err(QueryError::QueryPolicyRefused {
                policy: policy(),
                reason: message,
                allowed: alternatives,
            }),
        }
    }

    /// The registry as the one context `ctx` sees it: every gate decides as it
    /// does under `ctx`, and the result takes no context. Twin of
    /// `QueryRegistry.under`.
    pub fn under(&self, ctx: C) -> QueryRegistry<()>
    where
        C: Send + Sync + 'static,
    {
        let ctx = Arc::new(ctx);
        QueryRegistry {
            entries: self.entries.clone(),
            shapes: self.shapes.clone(),
            gates: self.gates.iter().map(|g| g.under(&ctx)).collect(),
        }
    }
}
