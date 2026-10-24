//! The canonical-form declarations of an IDL vocabulary, and the IDL-driven node
//! codec that applies them: a document is decoded against the vocabulary and
//! re-encoded, and the bytes that come out are its canonical form.
//!
//! Three declarations decide that form:
//!
//! - a member's ALIASES — retired spellings a decoder accepts and normalises to
//!   the canonical name, the canonical spelling winning when both are present,
//!   otherwise the first alias in declared order; and an enum case's aliases,
//!   lenient wire strings that decode to the case and re-encode as its canonical
//!   string;
//! - a member's PRESENCE condition — `whenPresent` / `whenAbsent` a sibling, or
//!   `whenEquals` a sibling's scalar value (an absent `omitDefault` sibling holding
//!   its default). A member whose condition fails carries no meaning, so the
//!   decoder drops it and the encoder never writes it;
//! - an `omitDefault` member's default literal — the empty map, a JSON value, or
//!   any scalar — omitted when the value equals it, compared by canonical bytes.
//!
//! **The vocabulary this module reads is the canonical-form slice of `idl.json`**:
//! node kinds over the types `str`, `int`, `bool`, `json`, `enum`, `list` and
//! `map`, with every optionality, alias and presence declaration, and enums with
//! their case aliases, under the default wire shape (the nested `id` + `kind`
//! envelope, `$type` discriminator, ordinal key order). A vocabulary declaring
//! anything outside that slice — unions, records, ops, a node envelope, another
//! wire shape, a type not listed — is refused by [`Vocabulary::from_json`] rather
//! than read partially.
//!
//! Twin of `Decode.decode` then `Encode.encode` (`Fuaran.Core.Idl`) and the
//! member questions of `WireForm` they share; certified against Core's
//! `laws/canonical-form-laws.json`.

use alloc::boxed::Box;

use crate::decode::{self, Cursor, DecodeCode, DecodeError, Value};
use crate::json::{JVal, JsonLimits, PathSeg, ordinal_cmp, render_canonical};
use crate::prelude::*;

/// The discriminator of the default wire shape.
const DISCRIMINATOR: &str = "$type";

// ─── The vocabulary ──────────────────────────────────────────────────────────

/// A declared member type. Twin of the `IdlType` cases this slice reads.
#[derive(Debug, Clone, PartialEq)]
pub enum IdlType {
    Str,
    Int,
    Bool,
    /// Any JSON, verbatim.
    Json,
    /// A declared enum, by name.
    Enum(String),
    List(Box<IdlType>),
    /// A map from string keys to values of the type.
    Map(Box<IdlType>),
}

/// An authored value. Twin of the `IdlValue` cases this slice reads; an enum
/// value carries its WIRE string.
#[derive(Debug, Clone, PartialEq)]
pub enum IdlValue {
    Str(String),
    Int(i32),
    Bool(bool),
    Enum(String),
    Json(JVal),
    List(Vec<IdlValue>),
    Map(Vec<(String, IdlValue)>),
}

/// Whether a member is on the wire. Twin of `Optionality`.
#[derive(Debug, Clone, PartialEq)]
pub enum Optionality {
    Required,
    Optional,
    /// Never on the wire.
    HostOnly,
    /// Omitted at its identity default, which an absent member decodes to.
    OmitDefault(IdlValue),
}

/// A member's presence condition. Twin of `PresenceCondition`.
#[derive(Debug, Clone, PartialEq)]
pub enum Presence {
    WhenPresent(String),
    WhenAbsent(String),
    WhenEquals(String, IdlValue),
}

/// One declared member. Twin of `IdlField`.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub name: String,
    pub ty: IdlType,
    pub opt: Optionality,
    /// Retired spellings, in priority order after the canonical name.
    pub aliases: Vec<String>,
    pub presence: Option<Presence>,
}

/// A node kind: its tag and its members in declared order. Twin of `IdlKind`.
#[derive(Debug, Clone, PartialEq)]
pub struct Kind {
    pub tag: String,
    pub fields: Vec<Field>,
}

/// An enum: its wire cases and each case's lenient spellings. Twin of `IdlEnum`.
#[derive(Debug, Clone, PartialEq)]
pub struct Enum {
    pub name: String,
    pub cases: Vec<String>,
    /// Keyed by the case's wire string.
    pub aliases: Vec<(String, Vec<String>)>,
}

impl Enum {
    /// The wire case a string decodes to: the case itself, or the case an alias
    /// names. Twin of `CaseAccepting` + `WireOf`.
    pub fn case_accepting(&self, s: &str) -> Option<&str> {
        if let Some(c) = self.cases.iter().find(|c| *c == s) {
            return Some(c);
        }
        self.aliases
            .iter()
            .find(|(_, spellings)| spellings.iter().any(|a| a == s))
            .and_then(|(case, _)| self.cases.iter().find(|c| *c == case))
            .map(String::as_str)
    }

    fn aliases_of(&self, case: &str) -> &[String] {
        self.aliases
            .iter()
            .find(|(c, _)| c == case)
            .map(|(_, a)| a.as_slice())
            .unwrap_or(&[])
    }
}

/// The canonical-form slice of a vocabulary.
#[derive(Debug, Clone, PartialEq)]
pub struct Vocabulary {
    pub kinds: Vec<Kind>,
    pub enums: Vec<Enum>,
}

/// A decoded node: its id, its kind's tag and its members in declared order.
/// Twin of `VNode`.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: String,
    pub kind: String,
    pub fields: Vec<(String, IdlValue)>,
}

/// Why a document has no canonical form: its decode refusal, or the encoder's.
#[derive(Debug, Clone, PartialEq)]
pub enum CanonicalFormError {
    Decode(DecodeError),
    Encode(String),
}

// ─── Reading idl.json ────────────────────────────────────────────────────────

type VResult<T> = Result<T, String>;

fn member<'a>(v: &'a JVal, k: &str, at: &str) -> VResult<&'a JVal> {
    v.field(k)
        .ok_or_else(|| format!("{at}: missing member '{k}'"))
}

fn string<'a>(v: &'a JVal, k: &str, at: &str) -> VResult<&'a str> {
    match member(v, k, at)? {
        JVal::Str(s) => Ok(s),
        _ => Err(format!("{at}: '{k}' is not a string")),
    }
}

fn array<'a>(v: Option<&'a JVal>, k: &str, at: &str) -> VResult<&'a [JVal]> {
    match v {
        None => Ok(&[]),
        Some(JVal::Arr(xs)) => Ok(xs),
        Some(_) => Err(format!("{at}: '{k}' is not an array")),
    }
}

fn strings(v: Option<&JVal>, k: &str, at: &str) -> VResult<Vec<String>> {
    array(v, k, at)?
        .iter()
        .map(|x| match x {
            JVal::Str(s) => Ok(s.clone()),
            _ => Err(format!("{at}: '{k}' holds a non-string")),
        })
        .collect()
}

fn tag_of<'a>(v: &'a JVal, at: &str) -> VResult<&'a str> {
    string(v, DISCRIMINATOR, at)
}

fn read_type(v: &JVal, at: &str) -> VResult<IdlType> {
    Ok(match tag_of(v, at)? {
        "str" => IdlType::Str,
        "int" => IdlType::Int,
        "bool" => IdlType::Bool,
        "json" => IdlType::Json,
        "enum" => IdlType::Enum(string(v, "name", at)?.to_string()),
        "list" => IdlType::List(Box::new(read_type(member(v, "of", at)?, at)?)),
        "map" => IdlType::Map(Box::new(read_type(member(v, "values", at)?, at)?)),
        other => {
            return Err(format!(
                "{at}: type '{other}' is outside the canonical-form slice"
            ));
        }
    })
}

fn read_value(v: &JVal, at: &str) -> VResult<IdlValue> {
    let named = |k: &str| -> VResult<Vec<(String, IdlValue)>> {
        array(Some(member(v, k, at)?), k, at)?
            .iter()
            .map(|e| {
                Ok((
                    string(e, "name", at)?.to_string(),
                    read_value(member(e, "value", at)?, at)?,
                ))
            })
            .collect()
    };
    Ok(match tag_of(v, at)? {
        "str" => IdlValue::Str(string(v, "value", at)?.to_string()),
        "int" => match member(v, "value", at)? {
            JVal::Num(n)
                if *n >= f64::from(i32::MIN)
                    && *n <= f64::from(i32::MAX)
                    && f64::from(*n as i32) == *n =>
            {
                IdlValue::Int(*n as i32)
            }
            _ => return Err(format!("{at}: an int value is not an int")),
        },
        "bool" => match member(v, "value", at)? {
            JVal::Bool(b) => IdlValue::Bool(*b),
            _ => return Err(format!("{at}: a bool value is not a bool")),
        },
        "enum" => IdlValue::Enum(string(v, "case", at)?.to_string()),
        "json" => IdlValue::Json(member(v, "value", at)?.clone()),
        "list" => IdlValue::List(
            array(Some(member(v, "items", at)?), "items", at)?
                .iter()
                .map(|x| read_value(x, at))
                .collect::<VResult<_>>()?,
        ),
        "map" => IdlValue::Map(named("entries")?),
        other => {
            return Err(format!(
                "{at}: value '{other}' is outside the canonical-form slice"
            ));
        }
    })
}

fn read_field(v: &JVal, kind: &str) -> VResult<Field> {
    let name = string(v, "name", kind)?.to_string();
    let at = format!("{kind}.{name}");
    let opt_v = member(v, "optionality", &at)?;
    let opt = match tag_of(opt_v, &at)? {
        "required" => Optionality::Required,
        "optional" => Optionality::Optional,
        "hostOnly" => Optionality::HostOnly,
        "omitDefault" => Optionality::OmitDefault(read_value(member(opt_v, "default", &at)?, &at)?),
        other => return Err(format!("{at}: unknown optionality '{other}'")),
    };
    let presence = match v.field("presence") {
        None => None,
        Some(p) => {
            let sibling = string(p, "sibling", &at)?.to_string();
            Some(match tag_of(p, &at)? {
                "whenPresent" => Presence::WhenPresent(sibling),
                "whenAbsent" => Presence::WhenAbsent(sibling),
                "whenEquals" => {
                    Presence::WhenEquals(sibling, read_value(member(p, "value", &at)?, &at)?)
                }
                other => return Err(format!("{at}: unknown presence '{other}'")),
            })
        }
    };
    Ok(Field {
        ty: read_type(member(v, "type", &at)?, &at)?,
        opt,
        aliases: strings(v.field("aliases"), "aliases", &at)?,
        presence,
        name,
    })
}

impl Vocabulary {
    /// Read the canonical-form slice of an `idl.json` document (twin of the
    /// `Artifact` reader's kinds, fields and enums), refusing a vocabulary that
    /// declares anything outside it.
    pub fn from_json(v: &JVal) -> Result<Vocabulary, String> {
        for k in ["unions", "records", "nodeFields", "ops"] {
            if !array(v.field(k), k, "vocabulary")?.is_empty() {
                return Err(format!(
                    "vocabulary: '{k}' is outside the canonical-form slice"
                ));
            }
        }
        if v.field("wire").is_some() {
            return Err(
                "vocabulary: a declared wire shape is outside the canonical-form slice".into(),
            );
        }
        let kinds = array(v.field("kinds"), "kinds", "vocabulary")?
            .iter()
            .map(|k| {
                let tag = string(k, "tag", "kind")?.to_string();
                let fields = array(k.field("fields"), "fields", &tag)?
                    .iter()
                    .map(|f| read_field(f, &tag))
                    .collect::<VResult<_>>()?;
                Ok(Kind { tag, fields })
            })
            .collect::<VResult<_>>()?;
        let enums = array(v.field("enums"), "enums", "vocabulary")?
            .iter()
            .map(|e| {
                let name = string(e, "name", "enum")?.to_string();
                let aliases = match e.field("aliases") {
                    None => Vec::new(),
                    Some(JVal::Obj(entries)) => entries
                        .iter()
                        .map(|(case, a)| Ok((case.clone(), strings(Some(a), "aliases", &name)?)))
                        .collect::<VResult<_>>()?,
                    Some(_) => return Err(format!("{name}: 'aliases' is not an object")),
                };
                Ok(Enum {
                    cases: strings(e.field("cases"), "cases", &name)?,
                    aliases,
                    name,
                })
            })
            .collect::<VResult<_>>()?;
        Ok(Vocabulary { kinds, enums })
    }

    fn enum_named(&self, name: &str) -> Option<&Enum> {
        self.enums.iter().find(|e| e.name == name)
    }

    fn kind_tagged(&self, tag: &str) -> Option<&Kind> {
        self.kinds.iter().find(|k| k.tag == tag)
    }
}

// ─── The member questions ────────────────────────────────────────────────────

fn field_named<'a>(fields: &'a [Field], name: &str) -> Option<&'a Field> {
    fields.iter().find(|f| f.name == name)
}

/// The member `f` in a wire object, under its spellings: the canonical name
/// wins, otherwise the first alias present. Twin of `WireForm.memberAt`.
fn member_at<'a>(f: &Field, members: &'a [(String, Value)]) -> Option<(&'a str, &'a Value)> {
    core::iter::once(&f.name)
        .chain(f.aliases.iter())
        .find_map(|n| members.iter().find(|(k, _)| k == n))
        .map(|(k, v)| (k.as_str(), v))
}

/// Whether the wire value `j` decodes to the scalar `v` at `t` — its canonical
/// encoding, or for an enum any alias of its case. Twin of
/// `WireForm.scalarSpellings`.
fn spells_scalar(vocab: &Vocabulary, t: &IdlType, v: &IdlValue, j: &Value) -> bool {
    match (t, v, j) {
        (IdlType::Str, IdlValue::Str(s), Value::Str(w)) => s == w,
        (IdlType::Int, IdlValue::Int(i), Value::Int(w)) => i == w,
        (IdlType::Bool, IdlValue::Bool(b), Value::Bool(w)) => b == w,
        (IdlType::Enum(name), IdlValue::Enum(case), Value::Str(w)) => {
            match vocab
                .enum_named(name)
                .and_then(|e| Some((e, e.case_accepting(case)?)))
            {
                Some((e, c)) => c == w || e.aliases_of(c).iter().any(|a| a == w),
                None => false,
            }
        }
        _ => false,
    }
}

/// Whether a presence condition holds over a wire object's members. Twin of
/// `WireForm.holdsOnWire`.
fn holds_on_wire(
    vocab: &Vocabulary,
    fields: &[Field],
    c: &Presence,
    members: &[(String, Value)],
) -> bool {
    let held = |name: &str| {
        field_named(fields, name).and_then(|f| member_at(f, members).map(|(_, j)| (f, j)))
    };
    match c {
        Presence::WhenPresent(s) => held(s).is_some(),
        Presence::WhenAbsent(s) => held(s).is_none(),
        Presence::WhenEquals(s, v) => match (held(s), field_named(fields, s)) {
            (Some((f, j)), _) => spells_scalar(vocab, &f.ty, v, j),
            (
                None,
                Some(Field {
                    opt: Optionality::OmitDefault(d),
                    ..
                }),
            ) => d == v,
            (None, _) => false,
        },
    }
}

/// Whether a presence condition holds over authored members; an absent
/// `omitDefault` sibling holds its default. Twin of `WireForm.holdsAuthored`.
fn holds_authored(fields: &[Field], c: &Presence, authored: &[(String, IdlValue)]) -> bool {
    let held = |name: &str| -> Option<&IdlValue> {
        authored
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v)
            .or_else(|| match field_named(fields, name) {
                Some(Field {
                    opt: Optionality::OmitDefault(d),
                    ..
                }) => Some(d),
                _ => None,
            })
    };
    match c {
        Presence::WhenPresent(s) => held(s).is_some(),
        Presence::WhenAbsent(s) => held(s).is_none(),
        Presence::WhenEquals(s, v) => held(s) == Some(v),
    }
}

/// The member `f` as the decoder reads it: under its spellings, and absent when
/// its presence condition fails. Twin of `WireForm.readMember`.
fn read_member<'a>(
    vocab: &Vocabulary,
    fields: &[Field],
    f: &Field,
    members: &'a [(String, Value)],
) -> Option<(&'a str, &'a Value)> {
    match &f.presence {
        Some(c) if !holds_on_wire(vocab, fields, c, members) => None,
        _ => member_at(f, members),
    }
}

// ─── Decode ──────────────────────────────────────────────────────────────────

fn object_members<'a>(c: &Cursor<'a>) -> Result<&'a [(String, Value)], DecodeError> {
    match c.value() {
        Value::Obj(m) => Ok(m),
        other => Err(c.fail(
            DecodeCode::WrongKind,
            format!("expected an object, found {}", other.kind_name()),
        )),
    }
}

fn verbatim(v: &Value) -> JVal {
    match v {
        Value::Str(s) => JVal::Str(s.clone()),
        Value::Int(i) => JVal::Num(f64::from(*i)),
        Value::Float(f) => JVal::Num(*f),
        Value::Bool(b) => JVal::Bool(*b),
        Value::Arr(xs) => JVal::Arr(xs.iter().map(verbatim).collect()),
        Value::Obj(ms) => JVal::Obj(ms.iter().map(|(k, v)| (k.clone(), verbatim(v))).collect()),
    }
}

/// Twin of `Decode.decodeValue` over this slice's types.
fn decode_value(vocab: &Vocabulary, t: &IdlType, c: &Cursor<'_>) -> Result<IdlValue, DecodeError> {
    Ok(match t {
        IdlType::Str => IdlValue::Str(c.string()?.to_string()),
        IdlType::Int => IdlValue::Int(c.int()?),
        IdlType::Bool => IdlValue::Bool(c.boolean()?),
        IdlType::Json => IdlValue::Json(verbatim(c.value())),
        IdlType::Enum(name) => {
            let s = c.string()?;
            let e = vocab.enum_named(name).ok_or_else(|| {
                c.fail(
                    DecodeCode::SchemaFault,
                    format!("enum '{name}' is not declared"),
                )
            })?;
            match e.case_accepting(s) {
                Some(case) => IdlValue::Enum(case.to_string()),
                None => {
                    return Err(c.fail(
                        DecodeCode::UnknownTag,
                        format!(
                            "case '{s}' of enum '{name}' is not one of {}",
                            e.cases.join(", ")
                        ),
                    ));
                }
            }
        }
        IdlType::List(inner) => IdlValue::List(
            c.items()?
                .iter()
                .map(|x| decode_value(vocab, inner, x))
                .collect::<Result<_, _>>()?,
        ),
        IdlType::Map(inner) => {
            let mut entries = Vec::new();
            for (k, _) in object_members(c)? {
                let at = c.field(k)?;
                entries.push((k.clone(), decode_value(vocab, inner, &at)?));
            }
            IdlValue::Map(entries)
        }
    })
}

/// Twin of `Decode.decodeFields`: each declared member in order, read under its
/// spellings and its presence condition. A refusal inside a member is at its
/// CANONICAL spelling, whichever spelling the document used.
fn decode_fields(
    vocab: &Vocabulary,
    fields: &[Field],
    obj: &Cursor<'_>,
) -> Result<Vec<(String, IdlValue)>, DecodeError> {
    let members = object_members(obj)?;
    let mut out = Vec::new();
    for f in fields {
        if f.opt == Optionality::HostOnly {
            continue;
        }
        match (read_member(vocab, fields, f, members), &f.opt) {
            (None, Optionality::OmitDefault(d)) => out.push((f.name.clone(), d.clone())),
            (None, Optionality::Required) => {
                let mut path = obj.path().to_vec();
                path.push(PathSeg::Key(f.name.clone()));
                return Err(DecodeError {
                    code: DecodeCode::MissingField,
                    path,
                    message: format!("the required member '{}' is absent", f.name),
                });
            }
            (None, _) => {}
            (Some((spelling, _)), _) => {
                let at = obj.field(spelling)?;
                let depth = obj.path().len();
                let v = decode_value(vocab, &f.ty, &at).map_err(|mut e| {
                    if let Some(seg) = e.path.get_mut(depth) {
                        *seg = PathSeg::Key(f.name.clone());
                    }
                    e
                })?;
                out.push((f.name.clone(), v));
            }
        }
    }
    Ok(out)
}

/// Decode a node document against the vocabulary: its `id`, then its `kind`
/// object read by its `$type` discriminator. Twin of `Decode.decodeDetailed`
/// under the nested envelope.
pub fn decode(vocab: &Vocabulary, text: &str) -> Result<Node, DecodeError> {
    let root = decode::parse(text, &JsonLimits::STRICT)?;
    let c = Cursor::root(&root);
    object_members(&c)?;
    let id = c.field("id")?.string()?.to_string();
    let kind_at = c.field("kind")?;
    object_members(&kind_at)?;
    let tags: Vec<&str> = vocab.kinds.iter().map(|k| k.tag.as_str()).collect();
    let tag = kind_at.tag(DISCRIMINATOR, &tags)?;
    let kind = vocab
        .kind_tagged(tag)
        .ok_or_else(|| kind_at.fail(DecodeCode::UnknownTag, format!("unknown kind '{tag}'")))?;
    Ok(Node {
        id,
        kind: tag.to_string(),
        fields: decode_fields(vocab, &kind.fields, &kind_at)?,
    })
}

// ─── Encode ──────────────────────────────────────────────────────────────────

/// Twin of `Encode.encodeValue` over this slice's types.
fn encode_value(vocab: &Vocabulary, t: &IdlType, v: &IdlValue) -> Result<JVal, String> {
    Ok(match (t, v) {
        (IdlType::Str, IdlValue::Str(s)) => JVal::Str(s.clone()),
        (IdlType::Int, IdlValue::Int(i)) => JVal::Num(f64::from(*i)),
        (IdlType::Bool, IdlValue::Bool(b)) => JVal::Bool(*b),
        (IdlType::Json, IdlValue::Json(j)) => j.clone(),
        (IdlType::Enum(name), IdlValue::Enum(case)) => match vocab.enum_named(name) {
            None => return Err(format!("unknown enum '{name}'")),
            Some(e) if e.cases.contains(case) => JVal::Str(case.clone()),
            Some(_) => return Err(format!("enum '{name}' has no case '{case}'")),
        },
        (IdlType::List(inner), IdlValue::List(xs)) => JVal::Arr(
            xs.iter()
                .map(|x| encode_value(vocab, inner, x))
                .collect::<Result<_, _>>()?,
        ),
        (IdlType::Map(inner), IdlValue::Map(entries)) => {
            let mut sorted: Vec<&(String, IdlValue)> = entries.iter().collect();
            sorted.sort_by(|a, b| ordinal_cmp(&a.0, &b.0));
            JVal::Obj(
                sorted
                    .into_iter()
                    .map(|(k, v)| Ok((k.clone(), encode_value(vocab, inner, v)?)))
                    .collect::<Result<_, String>>()?,
            )
        }
        _ => return Err(format!("authored value does not match IDL type {t:?}")),
    })
}

/// Twin of `Encode.encodeFields`: each declared member in order; a member whose
/// presence condition fails is never written, and an `omitDefault` member is
/// omitted when its canonical bytes are its default's.
fn encode_fields(
    vocab: &Vocabulary,
    fields: &[Field],
    authored: &[(String, IdlValue)],
) -> Result<Vec<(String, JVal)>, String> {
    let extra: Vec<&str> = authored
        .iter()
        .filter(|(n, _)| field_named(fields, n).is_none())
        .map(|(n, _)| n.as_str())
        .collect();
    if !extra.is_empty() {
        return Err(format!("authored fields not in IDL: {}", extra.join(", ")));
    }
    let mut out = Vec::new();
    for f in fields {
        if f.opt == Optionality::HostOnly {
            continue;
        }
        if f.presence
            .as_ref()
            .is_some_and(|c| !holds_authored(fields, c, authored))
        {
            continue;
        }
        let provided = authored.iter().find(|(n, _)| *n == f.name).map(|(_, v)| v);
        match (provided, &f.opt) {
            (None, Optionality::Required) => {
                return Err(format!("required field '{}' is absent", f.name));
            }
            (None, _) => {}
            (Some(v), Optionality::OmitDefault(d)) => {
                let j = encode_value(vocab, &f.ty, v)?;
                let at_default = v == d
                    || encode_value(vocab, &f.ty, d)
                        .is_ok_and(|jd| render_canonical(&j) == render_canonical(&jd));
                if !at_default {
                    out.push((f.name.clone(), j));
                }
            }
            (Some(v), _) => out.push((f.name.clone(), encode_value(vocab, &f.ty, v)?)),
        }
    }
    Ok(out)
}

/// Encode a node to its canonical bytes under the nested envelope. Twin of
/// `Encode.encode`.
pub fn encode(vocab: &Vocabulary, node: &Node) -> Result<String, String> {
    let kind = vocab
        .kind_tagged(&node.kind)
        .ok_or_else(|| format!("unknown kind '{}'", node.kind))?;
    let mut kind_fields = vec![(DISCRIMINATOR.to_string(), JVal::Str(node.kind.clone()))];
    kind_fields.extend(encode_fields(vocab, &kind.fields, &node.fields)?);
    let doc = JVal::Obj(vec![
        ("id".to_string(), JVal::Str(node.id.clone())),
        ("kind".to_string(), JVal::Obj(kind_fields)),
    ]);
    Ok(render_canonical(&doc))
}

/// A document's canonical form: [`decode()`] then [`encode()`]. Twin of
/// `CanonicalFormVectorExport.canonical`'s `Decode.decode` then `Encode.encode`.
pub fn canonical(vocab: &Vocabulary, text: &str) -> Result<String, CanonicalFormError> {
    let node = decode(vocab, text).map_err(CanonicalFormError::Decode)?;
    encode(vocab, &node).map_err(CanonicalFormError::Encode)
}
