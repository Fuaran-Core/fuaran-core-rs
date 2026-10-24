//! The columnar strand: the closed scalar type set, the cell, the typed column,
//! the table and its well-formedness check, the canonical `DataSource` wire codec
//! and the pinned aggregate semantics. The twin of the reference's
//! `Fuaran.Core.Column` (`ColumnType`, `TemporalText`, `Cell`, `Column`, `Table`
//! and `ColumnCodec`), certified against Core's `refusals/codec-refusals.json`
//! (the `column` vectors) and `laws/decimal-laws.json` (the codec and aggregate
//! cases).
//!
//! **The wire shape.** A source is `{"schema": [entry…], "columns": {name:
//! {"values": […], "validity": […]}}}` — column-oriented, a `values` array and a
//! `validity` mask per column — or `{"ref": name, "schema": []}`, a name the host
//! resolves. Bytes are canonical ([`crate::json::render_canonical`]): keys sorted
//! ordinally and the canonical number form, so a columnar payload is byte-identical
//! across hosts. The decoder reads the lenient shorthands the reference reads (a
//! bare array for an all-present column, a column object with no mask, an omitted
//! schema inferred from the cells) and normalises them on re-encode.
//!
//! **What a column holds.** A column of a type holds that type's elements and
//! nothing else, so a cell of another type cannot sit in one: [`Column::of_cells`]
//! refuses it. A date is held as its day count since 1970-01-01, an instant as its
//! epoch second and its fraction at the column's unit, a decimal as its text.

use core::cmp::Ordering;

use crate::decimal;
use crate::decode::{self, Value};
use crate::field::{self, Field};
use crate::json::{self, JVal, JsonError, JsonErrorKind, JsonLimit, JsonLimits, JsonRefusal};
use crate::number::format_finite_double;
use crate::prelude::*;

// ─── The scalar set ──────────────────────────────────────────────────────────

/// The resolution a timestamp column stores its instants at — twin of `TimeUnit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeUnit {
    Seconds,
    Milliseconds,
    Microseconds,
    Nanoseconds,
}

impl TimeUnit {
    /// How many fraction digits the unit holds: 0, 3, 6 or 9 (`TimeUnit.digits`).
    pub fn digits(self) -> u32 {
        match self {
            TimeUnit::Seconds => 0,
            TimeUnit::Milliseconds => 3,
            TimeUnit::Microseconds => 6,
            TimeUnit::Nanoseconds => 9,
        }
    }

    /// How many of the unit make one second (`TimeUnit.scale`).
    pub fn scale(self) -> i64 {
        10i64.pow(self.digits())
    }

    /// The coarsest unit holding `d` fraction digits exactly (`TimeUnit.ofDigits`).
    pub fn of_digits(d: u32) -> TimeUnit {
        match d {
            0 => TimeUnit::Seconds,
            1..=3 => TimeUnit::Milliseconds,
            4..=6 => TimeUnit::Microseconds,
            _ => TimeUnit::Nanoseconds,
        }
    }

    /// Does an instant held in `self` lose nothing held in `target`? (`TimeUnit.widens`)
    pub fn widens(self, target: TimeUnit) -> bool {
        self.digits() <= target.digits()
    }
}

/// The fixed scalar type set a column ranges over — twin of `ColumnType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnType {
    Int,
    Float,
    Bool,
    String,
    Date,
    Timestamp(TimeUnit),
    Decimal,
}

impl ColumnType {
    /// Every type in the reference's order (`ColumnType.all`): the `UnknownType`
    /// enumeration and the order of the scalar set.
    pub const ALL: [ColumnType; 10] = [
        ColumnType::Int,
        ColumnType::Float,
        ColumnType::Bool,
        ColumnType::String,
        ColumnType::Date,
        ColumnType::Timestamp(TimeUnit::Seconds),
        ColumnType::Decimal,
        ColumnType::Timestamp(TimeUnit::Milliseconds),
        ColumnType::Timestamp(TimeUnit::Microseconds),
        ColumnType::Timestamp(TimeUnit::Nanoseconds),
    ];

    /// Every type's wire tag, in [`ColumnType::ALL`]'s order.
    pub const TAGS: [&'static str; 10] = {
        let mut tags = [""; 10];
        let mut i = 0;
        while i < tags.len() {
            tags[i] = ColumnType::ALL[i].tag();
            i += 1;
        }
        tags
    };

    /// The canonical wire tag (`ColumnType.tag`).
    pub const fn tag(self) -> &'static str {
        match self {
            ColumnType::Int => "int",
            ColumnType::Float => "float",
            ColumnType::Bool => "bool",
            ColumnType::String => "string",
            ColumnType::Date => "date",
            ColumnType::Timestamp(TimeUnit::Seconds) => "timestamp",
            ColumnType::Timestamp(TimeUnit::Milliseconds) => "timestamp_ms",
            ColumnType::Timestamp(TimeUnit::Microseconds) => "timestamp_us",
            ColumnType::Timestamp(TimeUnit::Nanoseconds) => "timestamp_ns",
            ColumnType::Decimal => "decimal",
        }
    }

    /// The type a wire tag names, or `None` (`ColumnType.ofTag`).
    pub fn of_tag(tag: &str) -> Option<ColumnType> {
        ColumnType::ALL.into_iter().find(|t| t.tag() == tag)
    }

    /// The pinned widening lattice (`ColumnType.widens`): the identity, `Int` into
    /// `Float` or `Decimal`, and a coarser timestamp unit into a finer one.
    pub fn widens(self, target: ColumnType) -> bool {
        match (self, target) {
            (ColumnType::Int, ColumnType::Float | ColumnType::Decimal) => true,
            (ColumnType::Timestamp(a), ColumnType::Timestamp(b)) => a.widens(b),
            _ => self == target,
        }
    }
}

// ─── The canonical temporal text ─────────────────────────────────────────────

/// The canonical text of a date and a timestamp, and the integer forms a column
/// stores them in — twin of `TemporalText`. A date is exactly `YYYY-MM-DD`; a
/// timestamp `YYYY-MM-DDThh:mm:ssZ` or `YYYY-MM-DDThh:mm:ss.FZ` (UTC, `F` one to
/// nine digits with no trailing zero). Years `0000`–`9999`, the proleptic
/// Gregorian calendar, no leap second. Pure integer arithmetic.
pub mod temporal {
    use super::TimeUnit;
    use crate::prelude::*;

    /// The day count of `0000-01-01`.
    pub const MIN_DAY: i32 = -719_528;
    /// The day count of `9999-12-31`.
    pub const MAX_DAY: i32 = 2_932_896;
    /// The epoch second of `0000-01-01T00:00:00Z`.
    pub const MIN_SECOND: i64 = MIN_DAY as i64 * 86_400;
    /// The epoch second of `9999-12-31T23:59:59Z`.
    pub const MAX_SECOND: i64 = MAX_DAY as i64 * 86_400 + 86_399;

    fn digits_at(s: &[u8], from: usize, count: usize) -> Option<i64> {
        let mut acc = 0i64;
        for &c in &s[from..from + count] {
            if !c.is_ascii_digit() {
                return None;
            }
            acc = acc * 10 + i64::from(c - b'0');
        }
        Some(acc)
    }

    fn is_leap(y: i64) -> bool {
        y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
    }

    fn days_in(y: i64, m: i64) -> i64 {
        match m {
            2 => {
                if is_leap(y) {
                    29
                } else {
                    28
                }
            }
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        }
    }

    /// Days since 1970-01-01 of a civil date (`TemporalText.daysOfCivil`).
    pub fn days_of_civil(y: i64, m: i64, d: i64) -> i64 {
        let y = if m <= 2 { y - 1 } else { y };
        let y4 = y + 400;
        let era = y4 / 400;
        let yoe = y4 - era * 400;
        let mp = (m + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        (era - 1) * 146_097 + doe - 719_468
    }

    /// The civil date `(year, month, day)` of a day count (`TemporalText.civilOfDays`).
    pub fn civil_of_days(days: i64) -> (i64, i64, i64) {
        let z = days + 719_468 + 146_097;
        let era = z / 146_097;
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        (yoe + (era - 1) * 400 + i64::from(m <= 2), m, d)
    }

    fn date_part(s: &[u8]) -> Option<(i64, i64, i64)> {
        if s[4] != b'-' || s[7] != b'-' {
            return None;
        }
        let (y, m, d) = (
            digits_at(s, 0, 4)?,
            digits_at(s, 5, 2)?,
            digits_at(s, 8, 2)?,
        );
        ((1..=12).contains(&m) && d >= 1 && d <= days_in(y, m)).then_some((y, m, d))
    }

    /// The day count of a canonical date, or `None` (`TemporalText.tryDays`).
    pub fn try_days(s: &str) -> Option<i32> {
        // Every position of a canonical date is checked as an ASCII character, so a
        // text with any other character is refused here, and byte indices are the
        // reference's UTF-16 indices.
        let b = s.as_bytes();
        if !s.is_ascii() || b.len() != 10 {
            return None;
        }
        date_part(b).map(|(y, m, d)| days_of_civil(y, m, d) as i32)
    }

    /// Is `days` a day the canonical form spells (`TemporalText.isDayInRange`)?
    pub fn is_day_in_range(days: i32) -> bool {
        (MIN_DAY..=MAX_DAY).contains(&days)
    }

    /// The canonical text of a day count in range (`TemporalText.dateText`).
    pub fn date_text(days: i32) -> String {
        let (y, m, d) = civil_of_days(i64::from(days));
        format!("{y:04}-{m:02}-{d:02}")
    }

    fn fraction_part(s: &[u8]) -> Option<(u32, i64)> {
        let len = s.len();
        if len == 20 {
            return (s[19] == b'Z').then_some((0, 0));
        }
        if (22..=30).contains(&len) && s[19] == b'.' && s[len - 1] == b'Z' {
            let n = len - 21;
            let f = digits_at(s, 20, n)?;
            return (s[len - 2] != b'0').then_some((n as u32, f));
        }
        None
    }

    /// The floor epoch second of a canonical instant and its fraction as
    /// `(digits, value)`.
    fn parse_instant(s: &str) -> Option<(i64, u32, i64)> {
        let b = s.as_bytes();
        if !s.is_ascii() || b.len() < 20 || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
            return None;
        }
        let (y, m, d) = date_part(b)?;
        let (h, mi, se) = (
            digits_at(b, 11, 2)?,
            digits_at(b, 14, 2)?,
            digits_at(b, 17, 2)?,
        );
        let (n, f) = fraction_part(b)?;
        if h > 23 || mi > 59 || se > 59 {
            return None;
        }
        Some((
            days_of_civil(y, m, d) * 86_400 + h * 3600 + mi * 60 + se,
            n,
            f,
        ))
    }

    /// True where `s` is a canonical timestamp (`TemporalText.isCanonicalTimestamp`).
    pub fn is_canonical_timestamp(s: &str) -> bool {
        parse_instant(s).is_some()
    }

    /// The coarsest unit holding the canonical instant `s` exactly; seconds for any
    /// other text (`TemporalText.unitOf`).
    pub fn unit_of(s: &str) -> TimeUnit {
        parse_instant(s).map_or(TimeUnit::Seconds, |(_, n, _)| TimeUnit::of_digits(n))
    }

    /// An instant text's epoch second and its fraction scaled to `unit`, or `None`
    /// where it is not canonical or finer than the unit (`TemporalText.tryInstant`).
    pub fn try_instant(unit: TimeUnit, s: &str) -> Option<(i64, i64)> {
        let (second, n, f) = parse_instant(s)?;
        (n <= unit.digits()).then(|| (second, f * 10i64.pow(unit.digits() - n)))
    }

    /// Is a second and a fraction a valid instant in `unit`
    /// (`TemporalText.isInstantInRange`)?
    pub fn is_instant_in_range(unit: TimeUnit, second: i64, fraction: i64) -> bool {
        (MIN_SECOND..=MAX_SECOND).contains(&second) && (0..unit.scale()).contains(&fraction)
    }

    /// The minimal canonical text of an instant in range (`TemporalText.instantText`).
    pub fn instant_text(unit: TimeUnit, second: i64, fraction: i64) -> String {
        let day = second.div_euclid(86_400);
        let sod = second - day * 86_400;
        let frac = if fraction == 0 {
            String::new()
        } else {
            let width = unit.digits() as usize;
            let digits = format!("{fraction:0width$}");
            format!(".{}", digits.trim_end_matches('0'))
        };
        format!(
            "{}T{:02}:{:02}:{:02}{frac}Z",
            date_text(day as i32),
            sod / 3600,
            sod % 3600 / 60,
            sod % 60
        )
    }

    /// THE order of two timestamp texts (`TemporalText.compareInstants`):
    /// chronological over canonical texts, any other text after every canonical
    /// one, ordinally among its own kind.
    pub fn compare_instants(a: &str, b: &str) -> core::cmp::Ordering {
        match (is_canonical_timestamp(a), is_canonical_timestamp(b)) {
            (true, true) => {
                let frac = |s: &'_ str| -> String {
                    if s.len() == 20 {
                        String::new()
                    } else {
                        s[20..s.len() - 1].to_string()
                    }
                };
                a[..19].cmp(&b[..19]).then_with(|| frac(a).cmp(&frac(b)))
            }
            (true, false) => core::cmp::Ordering::Less,
            (false, true) => core::cmp::Ordering::Greater,
            (false, false) => crate::json::ordinal_cmp(a, b),
        }
    }
}

// ─── The cell ────────────────────────────────────────────────────────────────

/// One realized scalar cell — twin of `Cell`. `Null` is the absent cell; a date,
/// a timestamp and a decimal carry their canonical text.
#[derive(Debug, Clone, PartialEq)]
pub enum Cell {
    Int(i32),
    Float(f64),
    Bool(bool),
    Str(String),
    Date(String),
    Timestamp(String),
    Null,
    Decimal(String),
}

/// The column layer's total order over floats (`Cell.compareFloat`): IEEE order,
/// `-0 = 0`, and NaN one value above every other.
pub fn compare_float(a: f64, b: f64) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
    }
}

impl Cell {
    /// A `Decimal` cell of the canonical form of `text`, or `None` where it is not
    /// decimal text (`Cell.decimal`).
    pub fn decimal(text: &str) -> Option<Cell> {
        decimal::try_canonical(text).map(Cell::Decimal)
    }

    /// The type a present cell carries; `None` for `Null` (`Cell.typeOf`). A
    /// timestamp carries the coarsest unit that holds its instant.
    pub fn type_of(&self) -> Option<ColumnType> {
        Some(match self {
            Cell::Int(_) => ColumnType::Int,
            Cell::Float(_) => ColumnType::Float,
            Cell::Bool(_) => ColumnType::Bool,
            Cell::Str(_) => ColumnType::String,
            Cell::Date(_) => ColumnType::Date,
            Cell::Timestamp(s) => ColumnType::Timestamp(temporal::unit_of(s)),
            Cell::Decimal(_) => ColumnType::Decimal,
            Cell::Null => return None,
        })
    }

    fn as_float(&self) -> Option<f64> {
        match self {
            Cell::Int(i) => Some(f64::from(*i)),
            Cell::Float(f) => Some(*f),
            _ => None,
        }
    }

    fn as_decimal(&self) -> Option<String> {
        match self {
            Cell::Decimal(s) => decimal::try_canonical(s),
            Cell::Int(i) => Some(i.to_string()),
            _ => None,
        }
    }

    /// THE order between two present cells of one family, or `None` where they
    /// are incomparable (`Cell.compare`).
    pub fn compare(&self, other: &Cell) -> Option<Ordering> {
        use Cell::*;
        match (self, other) {
            (Int(_) | Float(_), Int(_) | Float(_)) => {
                Some(compare_float(self.as_float()?, other.as_float()?))
            }
            (Decimal(_) | Int(_), Decimal(_) | Int(_)) => {
                decimal::compare(&self.as_decimal()?, &other.as_decimal()?)
            }
            (Bool(x), Bool(y)) => Some(x.cmp(y)),
            (Str(x), Str(y)) | (Date(x), Date(y)) => Some(json::ordinal_cmp(x, y)),
            (Timestamp(x), Timestamp(y)) => Some(temporal::compare_instants(x, y)),
            _ => None,
        }
    }

    /// THE canonical identity token of a cell (`Cell.token`): `i:`, `f:`, `b:1` /
    /// `b:0`, `s:`, `d:`, `t:`, `m:` + the canonical decimal, `n:`.
    pub fn token(&self) -> String {
        match self {
            Cell::Int(i) => format!("i:{i}"),
            Cell::Float(f) => {
                let body = if f.is_nan() {
                    "NaN".to_string()
                } else if *f == f64::INFINITY {
                    "Inf".to_string()
                } else if *f == f64::NEG_INFINITY {
                    "-Inf".to_string()
                } else {
                    format_finite_double(*f)
                };
                format!("f:{body}")
            }
            Cell::Bool(b) => format!("b:{}", if *b { "1" } else { "0" }),
            Cell::Str(s) => format!("s:{s}"),
            Cell::Date(s) => format!("d:{s}"),
            Cell::Timestamp(s) => format!("t:{s}"),
            Cell::Decimal(s) => format!(
                "m:{}",
                decimal::try_canonical(s).unwrap_or_else(|| s.clone())
            ),
            Cell::Null => "n:".to_string(),
        }
    }
}

// ─── The codec envelope ──────────────────────────────────────────────────────

/// THE CODEC ENVELOPE — the closed set of refusals of the codec, of
/// [`Table::validate`] and of [`Column::of_cells`]; twin of `ColumnError`.
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnError {
    /// The input was not valid JSON (its depth cap as kind `MaxDepthExceeded`).
    NotJson(JsonError),
    /// A required member was absent.
    MissingField(String),
    /// A value of the wrong JSON shape, or a cell's text not its type's canonical form.
    MalformedShape(String),
    /// A `type` tag outside the scalar set; `expected` is [`ColumnType::ALL`].
    UnknownType {
        got: String,
        expected: Vec<ColumnType>,
    },
    /// A present cell's type does not widen into its column's declared type.
    TypeMismatch {
        column: String,
        expected: ColumnType,
        got: String,
    },
    /// A column's `values` and `validity` arrays had different lengths.
    LengthMismatch {
        column: String,
        values: usize,
        validity: usize,
    },
    /// A present float cell was non-finite.
    NonFiniteFloat { column: String, value: String },
    /// The table was structurally malformed (a duplicate or unmatched name).
    Malformed(String),
    /// `column` has `got` rows where the first column has `expected`.
    RaggedColumns {
        column: String,
        expected: usize,
        got: usize,
    },
}

impl ColumnError {
    /// The case's name, as the conformance vectors spell a refusal's class.
    pub fn class(&self) -> &'static str {
        match self {
            ColumnError::NotJson(_) => "NotJson",
            ColumnError::MissingField(_) => "MissingField",
            ColumnError::MalformedShape(_) => "MalformedShape",
            ColumnError::UnknownType { .. } => "UnknownType",
            ColumnError::TypeMismatch { .. } => "TypeMismatch",
            ColumnError::LengthMismatch { .. } => "LengthMismatch",
            ColumnError::NonFiniteFloat { .. } => "NonFiniteFloat",
            ColumnError::Malformed(_) => "Malformed",
            ColumnError::RaggedColumns { .. } => "RaggedColumns",
        }
    }

    /// A stable human sentence (`ColumnCodec.errorString`).
    pub fn message(&self) -> String {
        match self {
            ColumnError::NotJson(e) => {
                format!("not valid JSON: {} at position {}", e.message, e.offset)
            }
            ColumnError::MissingField(f) => format!("missing field: {f}"),
            ColumnError::MalformedShape(d) => format!("malformed: {d}"),
            ColumnError::UnknownType { got, expected } => {
                let tags: Vec<&str> = expected.iter().map(|t| t.tag()).collect();
                format!(
                    "unknown column type '{got}'; expected one of: {}",
                    tags.join(", ")
                )
            }
            ColumnError::TypeMismatch {
                column,
                expected,
                got,
            } => format!(
                "column '{column}': expected {} value, got {got}",
                expected.tag()
            ),
            ColumnError::LengthMismatch {
                column,
                values,
                validity,
            } => format!(
                "column '{column}': values/validity length mismatch ({values} vs {validity})"
            ),
            ColumnError::NonFiniteFloat { column, value } => format!(
                "column '{column}': non-finite float is not representable on the Fuaran wire: {value}"
            ),
            ColumnError::Malformed(d) => format!("malformed table: {d}"),
            ColumnError::RaggedColumns {
                column,
                expected,
                got,
            } => format!(
                "ragged table: column '{column}' has {got} rows where the first column has {expected}"
            ),
        }
    }
}

// ─── The typed column ────────────────────────────────────────────────────────

/// One stored element of a column — the type's storage at one row.
#[derive(Debug, Clone, PartialEq)]
enum Elem {
    Int(i32),
    Float(f64),
    Bool(bool),
    Str(String),
    /// Days since 1970-01-01.
    Date(i32),
    /// The floor epoch second and the fraction at the column's unit.
    Instant(i64, i64),
    /// The decimal text as held (canonical when it was decoded or built by `Cell::decimal`).
    Decimal(String),
}

/// Which rows of a column are present — twin of `Validity`: `AllValid` keeps no mask.
#[derive(Debug, Clone, PartialEq)]
pub enum Validity {
    AllValid,
    Mask(Vec<bool>),
}

/// A named, typed column — twin of `Column`. Built by [`Column::of_cells`] or the
/// codec's decode; read through [`Column::cell`].
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    name: String,
    ty: ColumnType,
    values: Vec<Elem>,
    validity: Validity,
}

fn outside(name: &str, ty: ColumnType, c: &Cell) -> Result<Option<Elem>, ColumnError> {
    match c.type_of() {
        Some(t) => Err(ColumnError::TypeMismatch {
            column: name.to_string(),
            expected: ty,
            got: t.tag().to_string(),
        }),
        None => Ok(None),
    }
}

fn shape(name: &str, what: &str) -> ColumnError {
    ColumnError::MalformedShape(format!("{name}: {what}"))
}

const DECIMAL_TEXT: &str = "a decimal value must be decimal text — an optional '-', digits, and an optional '.' followed by digits, with no exponent, sign '+', separator or white space";

/// Is `f` a whole number (`f = floor f`, without `std`'s `floor`)? Every double
/// of magnitude 2^52 or more is whole.
fn is_whole(f: f64) -> bool {
    f.is_finite() && (f.abs() >= 4_503_599_627_370_496.0 || (f as i64) as f64 == f)
}

/// The largest magnitude a whole-valued float token carries exactly: 2^53.
const INT53_MAX: f64 = 9_007_199_254_740_992.0;

/// An epoch number as the instant it names in a column at `unit` (the reference's
/// `epochInstant`): a seconds column reads milliseconds by magnitude (≥ 1e11,
/// truncated to the second); a sub-second column reads its own unit.
fn epoch_instant(name: &str, unit: TimeUnit, i: i64) -> Result<Elem, ColumnError> {
    let (second, fraction) = match unit {
        TimeUnit::Seconds => (
            if i.abs() >= 100_000_000_000 {
                i / 1000
            } else {
                i
            },
            0,
        ),
        _ => {
            let scale = unit.scale();
            let s = if i < 0 { i - scale + 1 } else { i } / scale;
            (s, i - s * scale)
        }
    };
    if temporal::is_instant_in_range(unit, second, fraction) {
        Ok(Elem::Instant(second, fraction))
    } else {
        Err(shape(
            name,
            &format!(
                "the epoch {i} names an instant outside the years 0000-9999 the canonical timestamp spells"
            ),
        ))
    }
}

fn not_canonical_text(name: &str, ty: ColumnType, form: &str) -> ColumnError {
    shape(
        name,
        &format!(
            "a {} value must be canonical ISO-8601 text, {form}, naming a moment that exists",
            ty.tag()
        ),
    )
}

impl ColumnType {
    /// The type's zero, held at an absent row.
    fn zero(self) -> Elem {
        match self {
            ColumnType::Int => Elem::Int(0),
            ColumnType::Float => Elem::Float(0.0),
            ColumnType::Bool => Elem::Bool(false),
            ColumnType::String => Elem::Str(String::new()),
            ColumnType::Date => Elem::Date(0),
            ColumnType::Timestamp(_) => Elem::Instant(0, 0),
            ColumnType::Decimal => Elem::Decimal("0".to_string()),
        }
    }

    /// A cell as the element a column of the type holds (`None` for `Null`) — the
    /// reference row's `OfCell`.
    fn of_cell(self, name: &str, c: &Cell) -> Result<Option<Elem>, ColumnError> {
        let elem = match (self, c) {
            (ColumnType::Int, Cell::Int(i)) => Elem::Int(*i),
            (ColumnType::Float, Cell::Float(f)) => Elem::Float(*f),
            (ColumnType::Float, Cell::Int(i)) => Elem::Float(f64::from(*i)),
            (ColumnType::Bool, Cell::Bool(b)) => Elem::Bool(*b),
            (ColumnType::String, Cell::Str(s)) => Elem::Str(s.clone()),
            (ColumnType::Date, Cell::Date(s)) => match temporal::try_days(s) {
                Some(d) => Elem::Date(d),
                None => {
                    return Err(shape(
                        name,
                        "a date cell must carry a canonical ISO-8601 date, YYYY-MM-DD, naming a day that exists",
                    ));
                }
            },
            (ColumnType::Timestamp(unit), Cell::Timestamp(s)) => {
                if !temporal::is_canonical_timestamp(s) {
                    return Err(shape(
                        name,
                        "a timestamp cell must carry a canonical ISO-8601 UTC timestamp, YYYY-MM-DDThh:mm:ssZ or YYYY-MM-DDThh:mm:ss.FZ with one to nine fraction digits and no trailing zero, naming an instant that exists",
                    ));
                }
                match temporal::try_instant(unit, s) {
                    Some((second, fraction)) => Elem::Instant(second, fraction),
                    None => return outside(name, self, c),
                }
            }
            (ColumnType::Decimal, Cell::Decimal(s)) => Elem::Decimal(s.clone()),
            (ColumnType::Decimal, Cell::Int(i)) => Elem::Decimal(i.to_string()),
            _ => return outside(name, self, c),
        };
        Ok(Some(elem))
    }

    /// One PRESENT JSON value as an element, or the refusal naming the column —
    /// the reference row's `ReadJson`.
    fn read_json(self, name: &str, v: &Value) -> Result<Elem, ColumnError> {
        let mismatch = || ColumnError::TypeMismatch {
            column: name.to_string(),
            expected: self,
            got: v.kind_name().to_string(),
        };
        match (self, v) {
            (ColumnType::Int, Value::Int(i)) => Ok(Elem::Int(*i)),
            (ColumnType::Float, Value::Float(f)) => Ok(Elem::Float(*f)),
            (ColumnType::Float, Value::Int(i)) => Ok(Elem::Float(f64::from(*i))),
            (ColumnType::Bool, Value::Bool(b)) => Ok(Elem::Bool(*b)),
            (ColumnType::String, Value::Str(s)) => Ok(Elem::Str(s.clone())),
            (ColumnType::Date, Value::Str(s)) => temporal::try_days(s)
                .map(Elem::Date)
                .ok_or_else(|| not_canonical_text(name, self, "YYYY-MM-DD")),
            (ColumnType::Timestamp(unit), Value::Str(s)) => match temporal::try_instant(unit, s) {
                Some((second, fraction)) => Ok(Elem::Instant(second, fraction)),
                None if temporal::is_canonical_timestamp(s) => Err(shape(
                    name,
                    &format!(
                        "a {} value's fraction is finer than the column's unit holds: {} or finer is needed",
                        self.tag(),
                        ColumnType::Timestamp(temporal::unit_of(s)).tag()
                    ),
                )),
                None => Err(not_canonical_text(
                    name,
                    self,
                    "YYYY-MM-DDThh:mm:ssZ or YYYY-MM-DDThh:mm:ss.FZ",
                )),
            },
            (ColumnType::Timestamp(unit), Value::Int(i)) => {
                epoch_instant(name, unit, i64::from(*i))
            }
            (ColumnType::Timestamp(unit), Value::Float(f)) if is_whole(*f) && f.abs() < 9e15 => {
                epoch_instant(name, unit, *f as i64)
            }
            (ColumnType::Decimal, Value::Int(i)) => Ok(Elem::Decimal(i.to_string())),
            (ColumnType::Decimal, Value::Float(f)) if is_whole(*f) && f.abs() <= INT53_MAX => {
                Ok(Elem::Decimal((*f as i64).to_string()))
            }
            (ColumnType::Decimal, Value::Str(s)) => decimal::try_canonical(s)
                .map(Elem::Decimal)
                .ok_or_else(|| shape(name, DECIMAL_TEXT)),
            _ => Err(mismatch()),
        }
    }

    /// One present JSON value read as a cell of the type and written back
    /// (`ColumnCodec.readCell` then `writeCell`), or `None` where a column of the
    /// type cannot carry it.
    pub(crate) fn round_trip_cell(self, v: &Value) -> Option<JVal> {
        self.read_json("", v).ok().map(|e| e.write_json(self))
    }

    /// The JSON value an absent row's slot carries — the reference row's `AbsentSlot`.
    fn absent_slot(self) -> JVal {
        match self {
            ColumnType::Int => JVal::Num(0.0),
            ColumnType::Float => JVal::Num(0.0),
            ColumnType::Bool => JVal::Bool(false),
            ColumnType::String | ColumnType::Date | ColumnType::Timestamp(_) => {
                JVal::Str(String::new())
            }
            ColumnType::Decimal => JVal::Str("0".to_string()),
        }
    }
}

impl Elem {
    /// The JSON value a present element is written as (`WriteJson`).
    fn write_json(&self, ty: ColumnType) -> JVal {
        match (self, ty) {
            (Elem::Int(i), _) => JVal::Num(f64::from(*i)),
            (Elem::Float(f), _) => JVal::Num(*f),
            (Elem::Bool(b), _) => JVal::Bool(*b),
            (Elem::Str(s) | Elem::Decimal(s), _) => JVal::Str(s.clone()),
            (Elem::Date(d), _) => JVal::Str(temporal::date_text(*d)),
            (Elem::Instant(s, f), ColumnType::Timestamp(unit)) => {
                JVal::Str(temporal::instant_text(unit, *s, *f))
            }
            (Elem::Instant(s, f), _) => {
                JVal::Str(temporal::instant_text(TimeUnit::Seconds, *s, *f))
            }
        }
    }

    /// The cell an element reads back as (`ToCell`).
    fn to_cell(&self, ty: ColumnType) -> Cell {
        match self {
            Elem::Int(i) => Cell::Int(*i),
            Elem::Float(f) => Cell::Float(*f),
            Elem::Bool(b) => Cell::Bool(*b),
            Elem::Str(s) => Cell::Str(s.clone()),
            Elem::Date(d) => Cell::Date(temporal::date_text(*d)),
            Elem::Instant(s, f) => {
                let unit = match ty {
                    ColumnType::Timestamp(u) => u,
                    _ => TimeUnit::Seconds,
                };
                Cell::Timestamp(temporal::instant_text(unit, *s, *f))
            }
            Elem::Decimal(s) => Cell::Decimal(s.clone()),
        }
    }

    /// The refusal of an element the codec cannot carry at row `row`, or `None`
    /// (`Uncarriable`).
    fn uncarriable(&self, name: &str, ty: ColumnType, row: usize) -> Option<ColumnError> {
        match (self, ty) {
            (Elem::Float(f), _) if !f.is_finite() => Some(ColumnError::NonFiniteFloat {
                column: name.to_string(),
                value: if f.is_nan() {
                    "NaN"
                } else if *f > 0.0 {
                    "Infinity"
                } else {
                    "-Infinity"
                }
                .to_string(),
            }),
            (Elem::Date(d), _) if !temporal::is_day_in_range(*d) => Some(shape(
                name,
                &format!(
                    "a date column holds days since 1970-01-01 from {} (0000-01-01) to {} (9999-12-31); {d} names no day the canonical form spells",
                    temporal::MIN_DAY,
                    temporal::MAX_DAY
                ),
            )),
            (Elem::Instant(s, f), ColumnType::Timestamp(unit))
                if !temporal::is_instant_in_range(unit, *s, *f) =>
            {
                Some(shape(
                    name,
                    &format!(
                        "a {} column holds whole epoch seconds from 0000-01-01T00:00:00Z to 9999-12-31T23:59:59Z and a fraction in [0, {}); row {row} holds an instant the canonical form cannot spell",
                        ty.tag(),
                        unit.scale()
                    ),
                ))
            }
            (Elem::Decimal(s), _) if !decimal::is_canonical(s) => Some(shape(
                name,
                "a decimal cell must carry canonical decimal text — an optional '-', integer digits with no leading zero, and a '.' with fraction digits only where the fraction is non-zero, with no trailing zero (build the cell with Cell.decimal)",
            )),
            _ => None,
        }
    }
}

impl Column {
    /// THE FILL over a row source: the type's zero at every absent row, and
    /// `AllValid` where no row is absent.
    fn fill<S>(
        name: &str,
        ty: ColumnType,
        source: &[S],
        mut read: impl FnMut(usize, &S) -> Result<Option<Elem>, ColumnError>,
    ) -> Result<Column, ColumnError> {
        let mut values = Vec::with_capacity(source.len());
        let mut mask = Vec::with_capacity(source.len());
        for (i, s) in source.iter().enumerate() {
            match read(i, s)? {
                Some(e) => {
                    values.push(e);
                    mask.push(true);
                }
                None => {
                    values.push(ty.zero());
                    mask.push(false);
                }
            }
        }
        let validity = if mask.iter().all(|p| *p) {
            Validity::AllValid
        } else {
            Validity::Mask(mask)
        };
        Ok(Column {
            name: name.to_string(),
            ty,
            values,
            validity,
        })
    }

    /// A column of `ty` from cells — twin of `Column.ofCells`: each cell widened as
    /// [`ColumnType::widens`] allows, `Null` the absent row; a cell of another type
    /// is the `TypeMismatch` naming it, and date or timestamp text that is not
    /// canonical the `MalformedShape` naming the form. Decimal text is held as
    /// found ([`Table::validate`] refuses a non-canonical one).
    pub fn of_cells(name: &str, ty: ColumnType, cells: &[Cell]) -> Result<Column, ColumnError> {
        Column::fill(name, ty, cells, |_, c| ty.of_cell(name, c))
    }

    /// The column's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The column's type.
    pub fn column_type(&self) -> ColumnType {
        self.ty
    }

    /// The row count.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// True for a column of no rows.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Whether row `i` is present.
    pub fn is_present(&self, i: usize) -> bool {
        match &self.validity {
            Validity::AllValid => i < self.values.len(),
            Validity::Mask(m) => m.get(i).copied().unwrap_or(false),
        }
    }

    /// The cell at row `i` — `Null` at an absent row or past the end (`Column.cell`).
    pub fn cell(&self, i: usize) -> Cell {
        if self.is_present(i) {
            self.values[i].to_cell(self.ty)
        } else {
            Cell::Null
        }
    }

    /// Every row as a cell, in order (`Column.toCells`).
    pub fn to_cells(&self) -> Vec<Cell> {
        (0..self.len()).map(|i| self.cell(i)).collect()
    }

    fn present_rows(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.len()).filter(|i| self.is_present(*i))
    }

    /// The `values` and `validity` arrays the codec writes for the column.
    fn json(&self) -> JVal {
        let values = (0..self.len())
            .map(|i| {
                if self.is_present(i) {
                    self.values[i].write_json(self.ty)
                } else {
                    self.ty.absent_slot()
                }
            })
            .collect();
        let validity = (0..self.len())
            .map(|i| JVal::Bool(self.is_present(i)))
            .collect();
        JVal::Obj(vec![
            ("values".to_string(), JVal::Arr(values)),
            ("validity".to_string(), JVal::Arr(validity)),
        ])
    }

    /// The first present element the codec cannot carry, as its refusal.
    fn first_uncarriable(&self) -> Option<ColumnError> {
        if let Validity::Mask(m) = &self.validity {
            if m.len() != self.len() {
                return Some(ColumnError::LengthMismatch {
                    column: self.name.clone(),
                    values: self.len(),
                    validity: m.len(),
                });
            }
        }
        self.present_rows()
            .find_map(|i| self.values[i].uncarriable(&self.name, self.ty, i))
    }
}

// ─── The table ───────────────────────────────────────────────────────────────

/// An embedded columnar table — twin of `Table`: the schema is the order
/// authority; the columns may be in any order and are matched to it by name.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub schema: Vec<Field>,
    pub columns: Vec<Column>,
}

/// The first name `names` carries twice, in order (`Table.firstDuplicate`).
pub fn first_duplicate<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let mut seen = BTreeSet::new();
    names.into_iter().find(|n| !seen.insert(*n))
}

impl Table {
    fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }

    /// Well-formedness — THE TABLE THE CODEC CAN CARRY (`Table.validate`), the
    /// first fault in this order: a duplicate schema or column name, an unmatched
    /// name (`Malformed`); a column type disagreeing with its entry
    /// (`TypeMismatch`); ragged columns (`RaggedColumns`); then column by column in
    /// schema order, row by row, a cell the codec cannot carry.
    pub fn validate(&self) -> Result<(), ColumnError> {
        let schema_names: Vec<&str> = self.schema.iter().map(|f| f.name()).collect();
        let column_names: Vec<&str> = self.columns.iter().map(|c| c.name.as_str()).collect();
        if let Some(n) = first_duplicate(schema_names.iter().copied()) {
            return Err(ColumnError::Malformed(format!(
                "duplicate schema name: {n}"
            )));
        }
        if let Some(n) = first_duplicate(column_names.iter().copied()) {
            return Err(ColumnError::Malformed(format!(
                "duplicate column name: {n}"
            )));
        }
        let missing: Vec<&str> = schema_names
            .iter()
            .copied()
            .filter(|n| !column_names.contains(n))
            .collect();
        if !missing.is_empty() {
            return Err(ColumnError::Malformed(format!(
                "schema names with no column: {}",
                missing.join(", ")
            )));
        }
        let extra: Vec<&str> = column_names
            .iter()
            .copied()
            .filter(|n| !schema_names.contains(n))
            .collect();
        if !extra.is_empty() {
            return Err(ColumnError::Malformed(format!(
                "columns absent from the schema: {}",
                extra.join(", ")
            )));
        }
        for f in &self.schema {
            if let Some(c) = self.column(f.name()) {
                if c.ty != f.column_type() {
                    return Err(ColumnError::TypeMismatch {
                        column: f.name().to_string(),
                        expected: f.column_type(),
                        got: c.ty.tag().to_string(),
                    });
                }
            }
        }
        if let Some((first, rest)) = self.columns.split_first() {
            if let Some(c) = rest.iter().find(|c| c.len() != first.len()) {
                return Err(ColumnError::RaggedColumns {
                    column: c.name.clone(),
                    expected: first.len(),
                    got: c.len(),
                });
            }
        }
        match self
            .schema
            .iter()
            .find_map(|f| self.column(f.name()).and_then(Column::first_uncarriable))
        {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }
}

// ─── The codec ───────────────────────────────────────────────────────────────

/// A data source — twin of `DataSource`: embedded columns, or a host-resolved
/// name.
#[derive(Debug, Clone, PartialEq)]
pub enum DataSource {
    Embedded(Table),
    Ref(String),
}

/// Encode a source to a value (`ColumnCodec.encodeJson`): embedded columns keyed
/// by name in schema order (a schema name with no column writes an empty one), or
/// a `ref` beside an empty `schema`.
pub fn encode_json(src: &DataSource) -> JVal {
    match src {
        DataSource::Embedded(t) => {
            let columns = t
                .schema
                .iter()
                .map(|f| {
                    let col = t.column(f.name()).map_or_else(
                        || {
                            JVal::Obj(vec![
                                ("values".to_string(), JVal::Arr(Vec::new())),
                                ("validity".to_string(), JVal::Arr(Vec::new())),
                            ])
                        },
                        Column::json,
                    );
                    (f.name().to_string(), col)
                })
                .collect();
            JVal::Obj(vec![
                (
                    "schema".to_string(),
                    JVal::Arr(t.schema.iter().map(field::field_json).collect()),
                ),
                ("columns".to_string(), JVal::Obj(columns)),
            ])
        }
        DataSource::Ref(r) => JVal::Obj(vec![
            ("schema".to_string(), JVal::Arr(Vec::new())),
            ("ref".to_string(), JVal::Str(r.clone())),
        ]),
    }
}

/// The canonical wire string for a source (`ColumnCodec.encode`). Assumes a table
/// [`Table::validate`] accepts; [`try_encode`] is the guarded entry point.
pub fn encode(src: &DataSource) -> String {
    json::render_canonical(&encode_json(src))
}

/// Total, guarded encode (`ColumnCodec.tryEncode`): [`Table::validate`], then
/// [`encode`]. A `ref` source always encodes.
pub fn try_encode(src: &DataSource) -> Result<String, ColumnError> {
    if let DataSource::Embedded(t) = src {
        t.validate()?;
    }
    Ok(encode(src))
}

/// The codec's projection of a read's refusal (`ColumnCodec.ofJsonRefusal`):
/// malformed text is `NotJson` with its class; the depth cap is `NotJson` with
/// kind `MaxDepthExceeded`; any other breach a `MalformedShape` naming it.
fn of_json_refusal(r: JsonRefusal) -> ColumnError {
    match r {
        JsonRefusal::Malformed(e) => ColumnError::NotJson(e),
        JsonRefusal::LimitExceeded(b) if b.limit == JsonLimit::Depth => {
            ColumnError::NotJson(JsonError {
                kind: JsonErrorKind::MaxDepthExceeded,
                offset: b.offset,
                message: format!("max nesting depth {} exceeded", b.value),
            })
        }
        JsonRefusal::LimitExceeded(b) => ColumnError::MalformedShape(format!(
            "limit {} of {} exceeded",
            b.limit.as_str(),
            b.value
        )),
    }
}

/// Read JSON text into the decode value model under the strict reading, the
/// refusal spelled in the codec's envelope.
pub fn read_value(text: &str) -> Result<Value, ColumnError> {
    decode::read_value(text, &JsonLimits::STRICT).map_err(of_json_refusal)
}

/// A member of an object, `None` when absent or when `v` is not an object
/// (`Decoder.tryMember`).
pub(crate) fn try_member<'a>(name: &str, v: &'a Value) -> Option<&'a Value> {
    match v {
        Value::Obj(members) => members.iter().find(|(k, _)| k == name).map(|(_, v)| v),
        _ => None,
    }
}

/// A required member: absent is `MissingField` naming it, a non-object the wrong kind.
pub(crate) fn get_field<'a>(name: &str, v: &'a Value) -> Result<&'a Value, ColumnError> {
    match v {
        Value::Obj(_) => {
            try_member(name, v).ok_or_else(|| ColumnError::MissingField(name.to_string()))
        }
        other => Err(ColumnError::MalformedShape(format!(
            "expected an object, got {}",
            other.kind_name()
        ))),
    }
}

fn ctx_shape(ctx: &str, message: String) -> ColumnError {
    ColumnError::MalformedShape(if ctx.is_empty() {
        message
    } else {
        format!("{ctx}: {message}")
    })
}

pub(crate) fn as_arr<'a>(ctx: &str, v: &'a Value) -> Result<&'a [Value], ColumnError> {
    match v {
        Value::Arr(items) => Ok(items),
        other => Err(ctx_shape(
            ctx,
            format!("expected an array, got {}", other.kind_name()),
        )),
    }
}

pub(crate) fn as_str<'a>(ctx: &str, v: &'a Value) -> Result<&'a str, ColumnError> {
    match v {
        Value::Str(s) => Ok(s),
        other => Err(ctx_shape(
            ctx,
            format!("expected a string, got {}", other.kind_name()),
        )),
    }
}

/// A column's `values` and `validity`: a bare array is all-present, and a column
/// object with no `validity` mask is all-present too.
fn column_parts<'a>(name: &str, col: &'a Value) -> Result<(&'a [Value], Vec<Value>), ColumnError> {
    let all_present = |xs: &[Value]| xs.iter().map(|_| Value::Bool(true)).collect();
    if let Value::Arr(xs) = col {
        return Ok((xs, all_present(xs)));
    }
    let values = as_arr(&format!("{name}.values"), get_field("values", col)?)?;
    match try_member("validity", col) {
        None => Ok((values, all_present(values))),
        Some(v) => Ok((values, as_arr(&format!("{name}.validity"), v)?.to_vec())),
    }
}

fn decode_column(columns: &Value, name: &str, ty: ColumnType) -> Result<Column, ColumnError> {
    let col = try_member(name, columns)
        .ok_or_else(|| ColumnError::MissingField(format!("columns.{name}")))?;
    let (values, validity) = column_parts(name, col)?;
    if values.len() != validity.len() {
        return Err(ColumnError::LengthMismatch {
            column: name.to_string(),
            values: values.len(),
            validity: validity.len(),
        });
    }
    Column::fill(name, ty, values, |i, v| match &validity[i] {
        Value::Bool(true) => ty.read_json(name, v).map(Some),
        Value::Bool(false) => Ok(None),
        p => Err(ColumnError::MalformedShape(format!(
            "{name}.validity: expected bool, got {}",
            p.kind_name()
        ))),
    })
}

/// One column's type from its cells, when the schema is omitted: all ints is
/// int, any fraction float, all bools bool, all strings string — never a date, a
/// timestamp or a decimal. An empty column, or mixed kinds, is refused.
fn infer_column_type(name: &str, values: &[Value]) -> Result<ColumnType, ColumnError> {
    if values.is_empty() {
        return Err(ColumnError::MalformedShape(format!(
            "{name}: cannot infer a column type from an empty / all-null column — declare it in an explicit \"schema\" array"
        )));
    }
    let mut tags: Vec<&str> = Vec::new();
    for v in values {
        let tag = match v {
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Bool(_) => "bool",
            Value::Str(_) => "string",
            _ => "other",
        };
        if !tags.contains(&tag) {
            tags.push(tag);
        }
    }
    match tags.as_slice() {
        ["int"] => Ok(ColumnType::Int),
        ["float"] | ["int", "float"] | ["float", "int"] => Ok(ColumnType::Float),
        ["bool"] => Ok(ColumnType::Bool),
        ["string"] => Ok(ColumnType::String),
        mixed => Err(ColumnError::MalformedShape(format!(
            "{name}: cannot infer a single column type from mixed cell kinds ({}) — declare it in an explicit \"schema\" array",
            mixed.join(", ")
        ))),
    }
}

/// Decode a source from a value (`ColumnCodec.decodeJson`). The `schema` is read
/// first (and may be omitted on an embedded source: inferred per column, in
/// ordinal key order); a `ref` member makes a `Ref`; otherwise each schema column
/// is read from `columns` and the table ENDS in [`Table::validate`]. Surplus
/// members are read past, and a key repeated in `columns` is refused.
pub fn decode_value(el: &Value) -> Result<DataSource, ColumnError> {
    let schema = match try_member("schema", el) {
        Some(s) => Some(field::decode_schema(s)?),
        None => None,
    };
    if let Some(r) = try_member("ref", el) {
        return Ok(DataSource::Ref(as_str("ref", r)?.to_string()));
    }
    let columns = get_field("columns", el)?;
    if let Value::Obj(members) = columns {
        if let Some(key) = first_duplicate(members.iter().map(|(k, _)| k.as_str())) {
            return Err(ColumnError::Malformed(format!(
                "duplicate column key in \"columns\": {key}"
            )));
        }
    }
    let schema = match schema {
        Some(s) => s,
        None => match columns {
            Value::Obj(members) => {
                let mut names: Vec<&str> = members.iter().map(|(k, _)| k.as_str()).collect();
                names.sort_by(|a, b| json::ordinal_cmp(a, b));
                let mut fields = Vec::with_capacity(names.len());
                for name in names {
                    let col = try_member(name, columns)
                        .ok_or_else(|| ColumnError::MissingField(format!("columns.{name}")))?;
                    let (values, _) = column_parts(name, col)?;
                    fields.push(Field::create(name, infer_column_type(name, values)?));
                }
                fields
            }
            _ => {
                return Err(ColumnError::MalformedShape(
                    "columns: expected object".to_string(),
                ));
            }
        },
    };
    let mut cols = Vec::with_capacity(schema.len());
    for f in &schema {
        cols.push(decode_column(columns, f.name(), f.column_type())?);
    }
    let table = Table {
        schema,
        columns: cols,
    };
    table.validate()?;
    Ok(DataSource::Embedded(table))
}

/// Decode a wire string into a source (`ColumnCodec.decode`).
pub fn decode(text: &str) -> Result<DataSource, ColumnError> {
    decode_value(&read_value(text)?)
}

// ─── The aggregates ──────────────────────────────────────────────────────────

/// A group/window aggregate function — twin of `AggFn`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggFn {
    Sum,
    Mean,
    Min,
    Max,
    Count,
    Median,
    StdDev,
    First,
    Last,
    CountDistinct,
}

impl AggFn {
    /// The reference's tag (`sum`, `countDistinct`, …).
    pub fn tag(self) -> &'static str {
        match self {
            AggFn::Sum => "sum",
            AggFn::Mean => "mean",
            AggFn::Min => "min",
            AggFn::Max => "max",
            AggFn::Count => "count",
            AggFn::Median => "median",
            AggFn::StdDev => "stddev",
            AggFn::First => "first",
            AggFn::Last => "last",
            AggFn::CountDistinct => "countDistinct",
        }
    }

    /// The function a tag names.
    pub fn of_tag(tag: &str) -> Option<AggFn> {
        use AggFn::*;
        [
            Sum,
            Mean,
            Min,
            Max,
            Count,
            Median,
            StdDev,
            First,
            Last,
            CountDistinct,
        ]
        .into_iter()
        .find(|f| f.tag() == tag)
    }

    fn keeps_numbers(self) -> bool {
        matches!(self, AggFn::StdDev | AggFn::Median)
    }
}

/// Why an aggregate was refused — twin of `AggregateError`.
#[derive(Debug, Clone, PartialEq)]
pub enum AggregateError {
    /// A numeric aggregate over a non-numeric column.
    IncompatibleAggType {
        agg: AggFn,
        col_type: ColumnType,
        expected: Vec<ColumnType>,
    },
    /// An answer outside its type's range.
    AggregateOverflow(String),
    /// A present decimal cell whose text is not decimal text.
    CellOutsideType {
        column: String,
        col_type: ColumnType,
        cell: String,
    },
}

impl AggregateError {
    /// The case's name, as the conformance vectors spell it.
    pub fn class(&self) -> &'static str {
        match self {
            AggregateError::IncompatibleAggType { .. } => "IncompatibleAggType",
            AggregateError::AggregateOverflow(_) => "AggregateOverflow",
            AggregateError::CellOutsideType { .. } => "CellOutsideType",
        }
    }
}

/// The float fold every float-valued aggregate shares.
struct FloatAcc {
    total: f64,
    numbers: usize,
    all_finite: bool,
    past_float: Option<AggregateError>,
    kept: Vec<f64>,
}

impl FloatAcc {
    fn new() -> FloatAcc {
        FloatAcc {
            total: 0.0,
            numbers: 0,
            all_finite: true,
            past_float: None,
            kept: Vec::new(),
        }
    }

    fn add(&mut self, keeps: bool, f: f64) {
        self.total += f;
        self.numbers += 1;
        if !f.is_finite() {
            self.all_finite = false;
        }
        if keeps {
            self.kept.push(f);
        }
    }
}

/// The mean and population standard deviation by a scaled Welford recurrence,
/// whose intermediates cannot overflow (the reference's `scaledMoments`).
fn scaled_moments(xs: &[f64]) -> (f64, f64) {
    let max_abs = xs
        .iter()
        .fold(0.0f64, |m, x| if x.abs() > m { x.abs() } else { m });
    let mut scale = 1.0;
    while max_abs * scale > 1.0 {
        scale *= 0.5;
    }
    let (mut mean, mut m2, mut k) = (0.0, 0.0, 0.0);
    for x in xs {
        k += 1.0;
        let y = x * scale;
        let d = y - mean;
        mean += d / k;
        m2 += d * (y - mean);
    }
    (mean / scale, sqrt(m2 / k) / scale)
}

/// The correctly rounded IEEE square root, without `std` (whose `f64::sqrt` is
/// what every host's statistic calls). Exact: the significand's root is taken as
/// an integer square root over `u128`, then rounded to nearest — a square root is
/// never exactly halfway between two doubles, so the rounding is unambiguous.
fn sqrt(x: f64) -> f64 {
    if x.is_nan() || x < 0.0 {
        return f64::NAN;
    }
    if x == 0.0 || x.is_infinite() {
        return x;
    }
    let bits = x.to_bits();
    let biased = ((bits >> 52) & 0x7ff) as i64;
    let frac = bits & ((1u64 << 52) - 1);
    // x = sig * 2^exp, with sig an integer.
    let (mut sig, mut exp) = if biased == 0 {
        (frac, -1074i64)
    } else {
        (frac | (1u64 << 52), biased - 1075)
    };
    // Normalise a subnormal so the significand has 53 bits.
    while sig < (1u64 << 52) {
        sig <<= 1;
        exp -= 1;
    }
    let mut sig = u128::from(sig);
    if exp % 2 != 0 {
        sig <<= 1;
        exp -= 1;
    }
    // sqrt(sig * 2^52) lies in [2^52, 2^53): 53 bits, the result's precision.
    let n = sig << 52;
    let mut q = isqrt(n);
    if n - q * q > q {
        q += 1;
    }
    // x's root is q * 2^(exp/2 - 26); q is exact as a double, and so is the scaling.
    scale_by_power_of_two(q as f64, exp / 2 - 26)
}

/// The integer square root (floor) of `n`, digit by digit.
fn isqrt(n: u128) -> u128 {
    let mut rem = n;
    let mut root = 0u128;
    let mut bit = 1u128 << 126;
    while bit > rem {
        bit >>= 2;
    }
    while bit != 0 {
        if rem >= root + bit {
            rem -= root + bit;
            root = (root >> 1) + bit;
        } else {
            root >>= 1;
        }
        bit >>= 2;
    }
    root
}

/// `x * 2^k` for a `k` whose power is a normal double, exactly.
fn scale_by_power_of_two(x: f64, k: i64) -> f64 {
    x * f64::from_bits(((k + 1023) as u64) << 52)
}

fn finite_or(col: &Column, what: &str, f: f64) -> Result<Cell, AggregateError> {
    if f.is_finite() {
        Ok(Cell::Float(f))
    } else {
        Err(AggregateError::AggregateOverflow(format!(
            "{}: {what} overflowed the float range",
            col.name
        )))
    }
}

fn float_answer(
    col: &Column,
    agg: AggFn,
    refold: impl FnOnce() -> FloatAcc,
    acc: FloatAcc,
) -> Result<Cell, AggregateError> {
    if let Some(e) = acc.past_float {
        return Err(e);
    }
    if acc.numbers == 0 && agg != AggFn::Median {
        return Ok(Cell::Null);
    }
    match agg {
        AggFn::Sum => {
            if acc.all_finite {
                finite_or(col, "sum", acc.total)
            } else {
                Ok(Cell::Float(acc.total))
            }
        }
        AggFn::Mean => {
            let mean = acc.total / acc.numbers as f64;
            if mean.is_finite() || !acc.all_finite {
                Ok(Cell::Float(mean))
            } else {
                finite_or(col, "mean", scaled_moments(&refold().kept).0)
            }
        }
        AggFn::StdDev => {
            let n = acc.numbers as f64;
            let mean = acc.total / n;
            let squares: f64 = acc
                .kept
                .iter()
                .fold(0.0, |s, x| s + (x - mean) * (x - mean));
            let deviation = sqrt(squares / n);
            if deviation.is_finite() || !acc.all_finite {
                Ok(Cell::Float(deviation))
            } else {
                finite_or(col, "stddev", scaled_moments(&acc.kept).1)
            }
        }
        _ => {
            let mut sorted = acc.kept;
            // A stable sort, so of two numbers the order calls equal the first stays first.
            sorted.sort_by(|a, b| compare_float(*a, *b));
            let n = sorted.len();
            if n == 0 {
                return Ok(Cell::Null);
            }
            let mid = n / 2;
            if n % 2 == 1 {
                return Ok(Cell::Float(sorted[mid]));
            }
            let (a, b) = (sorted[mid - 1], sorted[mid]);
            let plain = (a + b) / 2.0;
            if plain.is_finite() || !(a.is_finite() && b.is_finite()) {
                Ok(Cell::Float(plain))
            } else {
                finite_or(col, "median", a / 2.0 + b / 2.0)
            }
        }
    }
}

impl Column {
    /// The int and float numbers folded in row order.
    fn fold_numbers(&self, keeps: bool) -> FloatAcc {
        let mut acc = FloatAcc::new();
        for i in self.present_rows() {
            match &self.values[i] {
                Elem::Int(x) => acc.add(keeps, f64::from(*x)),
                Elem::Float(x) => acc.add(keeps, *x),
                _ => {}
            }
        }
        acc
    }

    /// `Min` / `Max`: `Min` keeps the first of equals, `Max` takes the last; a
    /// decimal answer is canonicalised.
    fn extreme(&self, is_min: bool) -> Cell {
        let replaces = |c: Ordering| {
            if is_min {
                c == Ordering::Greater
            } else {
                c != Ordering::Greater
            }
        };
        let mut found = Cell::Null;
        let mut any = false;
        for i in self.present_rows() {
            let c = self.cell(i);
            if !any {
                found = c;
                any = true;
                continue;
            }
            let order = match (&found, &c) {
                (Cell::Int(a), Cell::Int(b)) => Some(a.cmp(b)),
                (Cell::Float(a), Cell::Float(b)) => Some(compare_float(*a, *b)),
                (Cell::Str(a), Cell::Str(b)) => Some(json::ordinal_cmp(a, b)),
                _ => found.compare(&c),
            };
            if order.is_some_and(replaces) {
                found = c;
            }
        }
        match found {
            Cell::Decimal(s) => Cell::Decimal(decimal::try_canonical(&s).unwrap_or(s)),
            other => other,
        }
    }

    /// `CountDistinct`: distinct present values under [`Cell::token`]'s identity.
    fn distinct_count(&self) -> usize {
        self.present_rows()
            .map(|i| self.cell(i).token())
            .collect::<BTreeSet<String>>()
            .len()
    }

    /// The decimal admission walk: each present text canonicalised and handed to
    /// `step` with its row; the first text that is not decimal stops it as the
    /// named `CellOutsideType`.
    fn walk_decimals(&self, mut step: impl FnMut(usize, String)) -> Option<AggregateError> {
        for i in self.present_rows() {
            let text = match &self.values[i] {
                Elem::Decimal(s) => s,
                _ => continue,
            };
            match decimal::try_canonical(text) {
                Some(c) => step(i, c),
                None => {
                    return Some(AggregateError::CellOutsideType {
                        column: self.name.clone(),
                        col_type: self.ty,
                        cell: format!("decimal text '{text}' (not decimal)"),
                    });
                }
            }
        }
        None
    }

    fn aggregate_decimals(&self, agg: AggFn) -> Result<Cell, AggregateError> {
        let answer = |refusal: Option<AggregateError>,
                      k: &dyn Fn() -> Result<Cell, AggregateError>| {
            match refusal {
                Some(e) => Err(e),
                None => k(),
            }
        };
        let fold_decimals = |keeps: bool| {
            let mut acc = FloatAcc::new();
            let refusal = self.walk_decimals(|_, text| {
                if acc.past_float.is_none() {
                    match decimal::try_to_float(&text) {
                        Some(f) => acc.add(keeps, f),
                        None => {
                            acc.past_float = Some(AggregateError::AggregateOverflow(format!(
                                "{}: the decimal {text} is past the float range, and {} is a float",
                                self.name,
                                agg.tag()
                            )))
                        }
                    }
                }
            });
            (refusal, acc)
        };
        match agg {
            AggFn::Count => {
                let mut count = 0i32;
                let refusal = self.walk_decimals(|_, _| count += 1);
                answer(refusal, &|| Ok(Cell::Int(count)))
            }
            AggFn::First | AggFn::Last => {
                let row = if agg == AggFn::First {
                    Some(0)
                } else {
                    self.len().checked_sub(1)
                };
                let mut edge = Cell::Null;
                let refusal = self.walk_decimals(|i, text| {
                    if Some(i) == row {
                        edge = Cell::Decimal(text)
                    }
                });
                answer(refusal, &|| Ok(edge.clone()))
            }
            AggFn::Min | AggFn::Max | AggFn::CountDistinct => {
                let refusal = self.walk_decimals(|_, _| ());
                answer(refusal, &|| {
                    Ok(match agg {
                        AggFn::Min => self.extreme(true),
                        AggFn::Max => self.extreme(false),
                        _ => Cell::Int(self.distinct_count() as i32),
                    })
                })
            }
            AggFn::Sum => {
                // EXACT — never through a float.
                let mut total: Option<String> = None;
                let refusal = self.walk_decimals(|_, text| {
                    total = Some(match total.take() {
                        None => text,
                        Some(t) => decimal::add(&t, &text).unwrap_or(t),
                    })
                });
                answer(refusal, &|| {
                    Ok(total.clone().map_or(Cell::Null, Cell::Decimal))
                })
            }
            AggFn::Mean | AggFn::StdDev | AggFn::Median => {
                let (refusal, acc) = fold_decimals(agg.keeps_numbers());
                match refusal {
                    Some(e) => Err(e),
                    None => float_answer(self, agg, || fold_decimals(true).1, acc),
                }
            }
        }
    }

    /// One aggregate over the column with the pinned null, coercion and float
    /// semantics — twin of `Column.aggregate`. Nulls are skipped (`First` / `Last`
    /// keep them); a numeric aggregate over a non-numeric column is
    /// `IncompatibleAggType`; an int `Sum` is exact and range-checked; a decimal
    /// column is admitted first, its `Sum` exact and its float-valued aggregates
    /// read at each value's nearest double; no float aggregate answers an
    /// infinity over finite input; `StdDev` is the population form.
    pub fn aggregate(&self, agg: AggFn) -> Result<Cell, AggregateError> {
        let numeric = matches!(
            self.ty,
            ColumnType::Int | ColumnType::Float | ColumnType::Decimal
        );
        match agg {
            _ if self.ty == ColumnType::Decimal => self.aggregate_decimals(agg),
            AggFn::Count => Ok(Cell::Int(self.present_rows().count() as i32)),
            AggFn::First => Ok(self.cell(0)),
            AggFn::Last => Ok(self
                .len()
                .checked_sub(1)
                .map_or(Cell::Null, |i| self.cell(i))),
            AggFn::CountDistinct => Ok(Cell::Int(self.distinct_count() as i32)),
            AggFn::Min => Ok(self.extreme(true)),
            AggFn::Max => Ok(self.extreme(false)),
            _ if !numeric => Err(AggregateError::IncompatibleAggType {
                agg,
                col_type: self.ty,
                expected: vec![ColumnType::Int, ColumnType::Float, ColumnType::Decimal],
            }),
            AggFn::Sum if self.ty == ColumnType::Int => {
                let mut total = 0i64;
                let mut seen = false;
                for i in self.present_rows() {
                    if let Elem::Int(x) = self.values[i] {
                        total += i64::from(x);
                        seen = true;
                    }
                }
                if !seen {
                    Ok(Cell::Null)
                } else if let Ok(v) = i32::try_from(total) {
                    Ok(Cell::Int(v))
                } else {
                    Err(AggregateError::AggregateOverflow(format!(
                        "sum overflowed int32: {total}"
                    )))
                }
            }
            _ => float_answer(
                self,
                agg,
                || self.fold_numbers(true),
                self.fold_numbers(agg.keeps_numbers()),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_square_root_is_correctly_rounded() {
        assert_eq!(sqrt(4.0), 2.0);
        assert_eq!(sqrt(0.25), 0.5);
        assert_eq!(sqrt(2.0), core::f64::consts::SQRT_2);
        assert_eq!(sqrt(5e-324), 2.2227587494850775e-162);
        assert!(sqrt(-1.0).is_nan());
    }

    #[test]
    fn the_calendar_round_trips() {
        assert_eq!(temporal::try_days("1970-01-01"), Some(0));
        assert_eq!(temporal::try_days("0000-01-01"), Some(temporal::MIN_DAY));
        assert_eq!(temporal::try_days("9999-12-31"), Some(temporal::MAX_DAY));
        assert_eq!(temporal::date_text(temporal::MIN_DAY), "0000-01-01");
        assert_eq!(
            temporal::instant_text(TimeUnit::Milliseconds, -1, 500),
            "1969-12-31T23:59:59.5Z"
        );
        assert_eq!(
            temporal::try_instant(TimeUnit::Microseconds, "1969-12-31T23:59:59.5Z"),
            Some((-1, 500_000))
        );
    }
}
