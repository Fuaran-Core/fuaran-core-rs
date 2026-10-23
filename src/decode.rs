//! The decode error contract: the closed set of decode codes, each at the path it
//! names, and the reading combinators a host builds its decoders from.
//!
//! A decode of JSON text has two halves. The text is read by [`parse`], under a
//! [`JsonLimits`] record: malformed text is [`DecodeCode::InvalidJson`] at the
//! root, and a passed bound is [`DecodeCode::LimitExceeded`] at the path of the
//! container it was raised in. The value is then read through a [`Cursor`], whose
//! every refusal names its code and the path from the document root — so a
//! decoder written over it answers exactly as the reference's decoder of the same
//! shape does. Certified against Core's `decode/decode-rejects.json`.
//!
//! **The value model is the reference's null-free one**, [`Value`]: an integer
//! token within `i32` is [`Value::Int`], every other number [`Value::Float`], and a
//! `null` token is refused as malformed ([`crate::json::JsonErrorKind::NullNotRepresentable`]),
//! as the reference's strict reading refuses it. The distinction between the two
//! number cases is the TOKEN's, which is why this layer reads through its own
//! value model rather than over [`crate::json::JVal`]: `2.0` is not an `int`.

use crate::json::{Build, JsonLimits, JsonRefusal, PathSeg, scan};
use crate::prelude::*;

/// The reference's closed set of decode codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeCode {
    /// The text is not one well-formed JSON document (at the root).
    InvalidJson,
    /// A required member is absent (at the absent member's path).
    MissingField,
    /// A value of the wrong JSON kind.
    WrongKind,
    /// A discriminator names no case the reader knows (at the discriminator).
    UnknownTag,
    /// A number outside its declared range.
    OutOfRange,
    /// A member a closed object does not declare (at that member).
    UndeclaredMember,
    /// A bound passed (at the container it guards).
    LimitExceeded,
    /// A known case this position does not admit (at the discriminator).
    NotAdmitted,
    /// The decoder itself is ill-formed (an unresolved reference to a shape).
    SchemaFault,
}

impl DecodeCode {
    /// The reference's spelling (`InvalidJson`, …).
    pub fn as_str(self) -> &'static str {
        match self {
            DecodeCode::InvalidJson => "InvalidJson",
            DecodeCode::MissingField => "MissingField",
            DecodeCode::WrongKind => "WrongKind",
            DecodeCode::UnknownTag => "UnknownTag",
            DecodeCode::OutOfRange => "OutOfRange",
            DecodeCode::UndeclaredMember => "UndeclaredMember",
            DecodeCode::LimitExceeded => "LimitExceeded",
            DecodeCode::NotAdmitted => "NotAdmitted",
            DecodeCode::SchemaFault => "SchemaFault",
        }
    }
}

/// A refusal: its code, the path from the document root, and a sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeError {
    pub code: DecodeCode,
    pub path: Vec<PathSeg>,
    pub message: String,
}

impl DecodeError {
    /// The decode refusal of a refused read: malformed text is `InvalidJson` at the
    /// root, naming the parser's class; a breach is `LimitExceeded` at its path,
    /// naming the bound and its declared value.
    pub fn of_refusal(r: JsonRefusal) -> DecodeError {
        match r {
            JsonRefusal::Malformed(e) => DecodeError {
                code: DecodeCode::InvalidJson,
                path: Vec::new(),
                message: format!("{} at byte {}: {}", e.kind.as_str(), e.offset, e.message),
            },
            JsonRefusal::LimitExceeded(b) => DecodeError {
                code: DecodeCode::LimitExceeded,
                path: b.path,
                message: format!("limit {} of {} exceeded", b.limit.as_str(), b.value),
            },
        }
    }
}

/// A read value in the reference's null-free model.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Str(String),
    /// An integer token (no point, no exponent) within `i32`.
    Int(i32),
    /// Any other number: a point or an exponent, or an integer past `i32`.
    Float(f64),
    Bool(bool),
    Arr(Vec<Value>),
    Obj(Vec<(String, Value)>),
}

impl Value {
    /// The JSON kind, as every decode error names it.
    pub fn kind_name(&self) -> &'static str {
        match self {
            Value::Str(_) => "string",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Bool(_) => "bool",
            Value::Arr(_) => "array",
            Value::Obj(_) => "object",
        }
    }
}

struct ValueBuild;

impl Build for ValueBuild {
    type V = Value;
    fn null() -> Option<Value> {
        None
    }
    fn boolean(b: bool) -> Value {
        Value::Bool(b)
    }
    fn number(n: f64, integer_token: bool) -> Value {
        if integer_token && n >= f64::from(i32::MIN) && n <= f64::from(i32::MAX) {
            Value::Int(n as i32)
        } else {
            Value::Float(n)
        }
    }
    fn string(s: String) -> Value {
        Value::Str(s)
    }
    fn array(items: Vec<Value>) -> Value {
        Value::Arr(items)
    }
    fn object(members: Vec<(String, Value)>) -> Value {
        Value::Obj(members)
    }
}

/// Read JSON text under `limits` into the decode layer's value model.
pub fn parse(input: &str, limits: &JsonLimits) -> Result<Value, DecodeError> {
    scan::<ValueBuild>(input, limits).map_err(DecodeError::of_refusal)
}

/// A value and the path that reached it. Every reading answers a [`DecodeError`]
/// at that path, or at a member or item below it.
#[derive(Debug, Clone)]
pub struct Cursor<'a> {
    value: &'a Value,
    path: Vec<PathSeg>,
}

type DResult<T> = Result<T, DecodeError>;

impl<'a> Cursor<'a> {
    /// A cursor at the document root.
    pub fn root(value: &'a Value) -> Cursor<'a> {
        Cursor {
            value,
            path: Vec::new(),
        }
    }

    pub fn value(&self) -> &'a Value {
        self.value
    }

    pub fn path(&self) -> &[PathSeg] {
        &self.path
    }

    fn below(&self, seg: PathSeg, value: &'a Value) -> Cursor<'a> {
        let mut path = self.path.clone();
        path.push(seg);
        Cursor { value, path }
    }

    /// A refusal at this cursor's path.
    pub fn fail(&self, code: DecodeCode, message: impl Into<String>) -> DecodeError {
        DecodeError {
            code,
            path: self.path.clone(),
            message: message.into(),
        }
    }

    fn wrong_kind(&self, expected: &str) -> DecodeError {
        self.fail(
            DecodeCode::WrongKind,
            format!("expected {expected}, found {}", self.value.kind_name()),
        )
    }

    pub fn string(&self) -> DResult<&'a str> {
        match self.value {
            Value::Str(s) => Ok(s),
            _ => Err(self.wrong_kind("a string")),
        }
    }

    /// An integer token within `i32` only: `2.0`, `1e3` and an integer past `i32`
    /// are refused as `WrongKind`.
    pub fn int(&self) -> DResult<i32> {
        match self.value {
            Value::Int(i) => Ok(*i),
            _ => Err(self.wrong_kind("an int")),
        }
    }

    /// Any number.
    pub fn float(&self) -> DResult<f64> {
        match self.value {
            Value::Int(i) => Ok(f64::from(*i)),
            Value::Float(f) => Ok(*f),
            _ => Err(self.wrong_kind("a number")),
        }
    }

    pub fn boolean(&self) -> DResult<bool> {
        match self.value {
            Value::Bool(b) => Ok(*b),
            _ => Err(self.wrong_kind("a bool")),
        }
    }

    /// An int within `[lo, hi]`; outside it is `OutOfRange`.
    pub fn int_range(&self, lo: i32, hi: i32) -> DResult<i32> {
        let i = self.int()?;
        if i < lo || i > hi {
            return Err(self.fail(
                DecodeCode::OutOfRange,
                format!("{i} is outside [{lo}, {hi}]"),
            ));
        }
        Ok(i)
    }

    /// The items of an array, each at its index.
    pub fn items(&self) -> DResult<Vec<Cursor<'a>>> {
        match self.value {
            Value::Arr(items) => Ok(items
                .iter()
                .enumerate()
                .map(|(i, v)| self.below(PathSeg::Index(i), v))
                .collect()),
            _ => Err(self.wrong_kind("an array")),
        }
    }

    /// The items of an array of at most `max`; more is `LimitExceeded` at the array.
    pub fn items_at_most(&self, max: usize) -> DResult<Vec<Cursor<'a>>> {
        let items = self.items()?;
        if items.len() > max {
            return Err(self.fail(
                DecodeCode::LimitExceeded,
                format!("{} items where at most {max} are admitted", items.len()),
            ));
        }
        Ok(items)
    }

    fn members(&self) -> DResult<&'a [(String, Value)]> {
        match self.value {
            Value::Obj(m) => Ok(m),
            _ => Err(self.wrong_kind("an object")),
        }
    }

    /// An optional member: `None` when absent.
    pub fn optional(&self, name: &str) -> DResult<Option<Cursor<'a>>> {
        Ok(self
            .members()?
            .iter()
            .find(|(k, _)| k == name)
            .map(|(k, v)| self.below(PathSeg::Key(k.clone()), v)))
    }

    /// A required member; absent is `MissingField` at the absent member's path.
    pub fn field(&self, name: &str) -> DResult<Cursor<'a>> {
        match self.optional(name)? {
            Some(c) => Ok(c),
            None => {
                let mut path = self.path.clone();
                path.push(PathSeg::Key(name.to_string()));
                Err(DecodeError {
                    code: DecodeCode::MissingField,
                    path,
                    message: format!("the required member '{name}' is absent"),
                })
            }
        }
    }

    /// A closed object: the first member, in document order, that `declared` does
    /// not name is `UndeclaredMember` at that member. The reference checks this
    /// before it reads any declared member.
    pub fn closed(&self, declared: &[&str]) -> DResult<()> {
        for (k, v) in self.members()? {
            if !declared.contains(&k.as_str()) {
                return Err(self.below(PathSeg::Key(k.clone()), v).fail(
                    DecodeCode::UndeclaredMember,
                    format!("the member '{k}' is not declared here"),
                ));
            }
        }
        Ok(())
    }

    /// The discriminator of a tagged union read off the SAME object as its case:
    /// absent is `MissingField` and a non-string `WrongKind`, both at the key, and a
    /// tag outside `known` is `UnknownTag` there.
    pub fn tag(&self, key: &str, known: &[&str]) -> DResult<&'a str> {
        let at = self.field(key)?;
        let tag = at.string()?;
        if !known.contains(&tag) {
            return Err(at.fail(
                DecodeCode::UnknownTag,
                format!("'{tag}' is not one of {}", known.join(", ")),
            ));
        }
        Ok(tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json::JsonErrorKind;

    #[test]
    fn a_null_is_not_representable_and_the_number_token_decides_int() {
        let e = parse("null", &JsonLimits::STRICT).unwrap_err();
        assert_eq!(e.code, DecodeCode::InvalidJson);
        assert!(
            e.message
                .starts_with(JsonErrorKind::NullNotRepresentable.as_str())
        );
        let v = parse("[2, 2.0, 1e3, 3000000000]", &JsonLimits::STRICT).unwrap();
        assert_eq!(
            v,
            Value::Arr(vec![
                Value::Int(2),
                Value::Float(2.0),
                Value::Float(1000.0),
                Value::Float(3e9)
            ])
        );
        let items = Cursor::root(&v).items().unwrap();
        assert_eq!(items[0].int(), Ok(2));
        assert_eq!(items[1].int().unwrap_err().code, DecodeCode::WrongKind);
        assert_eq!(items[1].float(), Ok(2.0));
    }
}
