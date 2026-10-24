//! The seams' refusal and declaration documents, read by one declaration-driven
//! codec.
//!
//! The reference derives a codec for each seam union from its IDL declaration
//! (`CapabilitySeamCodec`, `PipelineSeamCodec`, `QuerySeamCodec`,
//! `FieldChangeSeamCodec`, `RejectionSeamCodec`): every case's members are read in
//! declaration order, the discriminator first, and the strict reading holds every
//! declared object to the members its declaration names before anything is read.
//! This module states those declarations as data — [`UnionDecl`], [`RecordDecl`],
//! [`Member`], [`Slot`] — and interprets them with ONE reader, [`read`], so a case
//! or member is added by editing a table, never by writing a decoder.
//!
//! What a read answers is the value's canonical re-encoding (the reference's
//! `encode<Name>` of what `decode<Name>` read), or the [`DecodeError`] the derived
//! codec answers, on code and path. Certified against Core's
//! `refusals/seam-refusals.json` (family `seamRefusals`).
//!
//! A slot the IDL does not model is HOSTED: its reader is the host's (a value
//! space, a node's output space, a column type, a predicate tag, a decode error, a
//! unit, a comparison's literal). Each is read here or by the module that owns
//! it: a unit by [`crate::unit`], a column type and a literal by [`crate::column`].

use crate::column::{ColumnType, temporal};
use crate::decode::{Cursor, DecodeCode, DecodeError, Value, quote_all};
use crate::json::{JVal, render_canonical};
use crate::prelude::*;
use crate::unit;

// ─── The declaration vocabulary ─────────────────────────────────────────────

/// The shape a member's value is read at — twin of the IDL's slot kinds as the
/// derived codecs read them (`dStr`, `dInt`, `dList`, an enum's decoder, a union's
/// or record's decoder, a type parameter's decoder, a hosted codec).
#[derive(Debug, Clone, Copy)]
pub enum Slot {
    /// A string (`dStr`): any other kind is `WrongKind`.
    Str,
    /// An integer token within `i32` (`dInt`).
    Int,
    /// Any number (`Decoder.float`, no non-finite sentinel): the value-space bounds.
    Number,
    /// The union's type parameter, read by the decoder the caller instantiates it
    /// at ([`Reading::param`]); the strict reading does not enter it.
    Param,
    /// An array, each item at its index (`dList`).
    List(&'static Slot),
    /// A closed string enumeration (an IDL enum): a miss is `UnknownTag` at the value.
    Enum(&'static EnumDecl),
    /// A discriminated union.
    Union(&'static UnionDecl),
    /// A record.
    Record(&'static RecordDecl),
    /// A slot whose reader is the host's.
    Hosted(Hosted),
}

/// A closed string enumeration (`IslandKind`, `SortDirection`, `NestRelation`).
#[derive(Debug)]
pub struct EnumDecl {
    pub name: &'static str,
    pub values: &'static [&'static str],
}

/// One declared member: its name, its slot, and whether it may be absent
/// (`dOpt`; the re-encoding then omits it) or is required (`dReq`).
#[derive(Debug)]
pub struct Member {
    pub name: &'static str,
    pub slot: Slot,
    pub optional: bool,
}

/// A case of a union: its tag and the members beside the discriminator. A case
/// with a case-level host codec (D198) reads its members through it; its
/// declared members still bound the strict reading.
#[derive(Debug)]
pub struct Case {
    pub tag: &'static str,
    pub members: &'static [Member],
    pub codec: Option<CaseCodec>,
}

/// A discriminated union: the discriminator's key (`$type`, or `member` for the
/// schema delta's `FieldChange`) and its cases.
#[derive(Debug)]
pub struct UnionDecl {
    pub name: &'static str,
    pub key: &'static str,
    pub cases: &'static [Case],
}

/// A record. `strict` is false for an anonymous tuple (a pipeline's argument
/// binding), which the derived strict reading does not enter.
#[derive(Debug)]
pub struct RecordDecl {
    pub name: &'static str,
    pub members: &'static [Member],
    pub strict: bool,
}

/// The hosted slots: values the IDL does not model, read by the host's codec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hosted {
    /// A value space, as `SpaceCodec.decoder` reads it.
    Space,
    /// A pipeline node's output space: `SpaceCodec.decoder`, then `Space.wellFormed`
    /// (twin of `PipelineWire.outputSpace`); an ill-formed space is `OutOfRange`.
    OutputSpace,
    /// A column type by its tag (twin of `QueryWire.columnTypeOf`).
    ColumnType,
    /// A predicate kind by its wire tag (twin of `QueryWire.predicateTagOf`).
    PredicateTag,
    /// A decode refusal in its JSON form (twin of `DecodeError.decoder`).
    DecodeError,
    /// A unit's text, read by the unit kit (`Unit.parse`, re-encoded by
    /// `Unit.render`); a text it does not parse is `OutOfRange`.
    Unit,
}

/// A case-level host codec (D198): the case's members are read by the host as one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseCodec {
    /// A comparison predicate: `column`, its literal's `type`, and its `value` read as
    /// one cell of that type (twin of `QueryWire.comparedOf` / `comparedMembers`).
    Compared,
}

const fn req(name: &'static str, slot: Slot) -> Member {
    Member {
        name,
        slot,
        optional: false,
    }
}

const fn opt(name: &'static str, slot: Slot) -> Member {
    Member {
        name,
        slot,
        optional: true,
    }
}

const fn case(tag: &'static str, members: &'static [Member]) -> Case {
    Case {
        tag,
        members,
        codec: None,
    }
}

const STRS: Slot = Slot::List(&Slot::Str);

// ─── The declarations ───────────────────────────────────────────────────────

/// The value space (`ValueSpace`) as `SpaceCodec` writes and reads it. Hosted, not
/// an IDL declaration: no strict reading enters it.
pub static VALUE_SPACE: UnionDecl = UnionDecl {
    name: "ValueSpace",
    key: "$type",
    cases: &[
        case("intRange", &[req("min", Slot::Int), req("max", Slot::Int)]),
        case(
            "floatRange",
            &[req("min", Slot::Number), req("max", Slot::Number)],
        ),
        case("stringLen", &[req("min", Slot::Int), req("max", Slot::Int)]),
        case("enum", &[req("values", STRS)]),
        case("anyString", &[]),
        case("slotTree", &[opt("slotKind", Slot::Str)]),
    ],
};

/// `IslandKind`.
pub static ISLAND_KIND: EnumDecl = EnumDecl {
    name: "IslandKind",
    values: &["pyodide", "fable", "js"],
};

/// `Placement`.
pub static PLACEMENT: UnionDecl = UnionDecl {
    name: "Placement",
    key: "$type",
    cases: &[
        case("buildTime", &[]),
        case("server", &[]),
        case("clientDeclarative", &[]),
        case("precomputed", &[]),
        case("clientIsland", &[req("island", Slot::Enum(&ISLAND_KIND))]),
    ],
};

/// `DeclarationFault`.
pub static DECLARATION_FAULT: UnionDecl = UnionDecl {
    name: "DeclarationFault",
    key: "$type",
    cases: &[
        case(
            "emptySpace",
            &[
                req("addr", Slot::Str),
                req("space", Slot::Hosted(Hosted::Space)),
            ],
        ),
        case("nonFiniteBound", &[req("addr", Slot::Str)]),
        case("duplicateHoleAddr", &[req("addr", Slot::Str)]),
        case("holeUnderSlot", &[req("node", Slot::Str)]),
    ],
};

/// `InvokeError`, the capability seam's refusal.
pub static INVOKE_ERROR: UnionDecl = UnionDecl {
    name: "InvokeError",
    key: "$type",
    cases: &[
        case(
            "noSuchCapability",
            &[req("id", Slot::Str), req("known", STRS)],
        ),
        case("duplicateCapability", &[req("id", Slot::Str)]),
        case(
            "unknownArg",
            &[req("addr", Slot::Str), req("declared", STRS)],
        ),
        case(
            "argOutOfSpace",
            &[
                req("addr", Slot::Str),
                req("space", Slot::Hosted(Hosted::Space)),
                req("got", Slot::Str),
            ],
        ),
        case("requiredArgsUnbound", &[req("addrs", STRS)]),
        case("uninvocableArg", &[req("addr", Slot::Str)]),
        case("bodyFailed", &[req("reason", Slot::Str)]),
        case(
            "nonTotalCapability",
            &[req("id", Slot::Str), req("addrs", STRS)],
        ),
        case(
            "illFormedCapability",
            &[
                req("id", Slot::Str),
                req("fault", Slot::Union(&DECLARATION_FAULT)),
            ],
        ),
        case("duplicateArg", &[req("addr", Slot::Str)]),
        case(
            "policyRefused",
            &[
                req("policy", Slot::Str),
                req("reason", Slot::Str),
                req("allowed", STRS),
            ],
        ),
        case("approvalRequired", &[req("policy", Slot::Str)]),
    ],
};

/// `Deferred<'T>`: its `ready` value is the type parameter.
pub static DEFERRED: UnionDecl = UnionDecl {
    name: "Deferred",
    key: "$type",
    cases: &[
        case("pending", &[]),
        case("ready", &[req("value", Slot::Param)]),
        case("failed", &[req("message", Slot::Str)]),
    ],
};

/// `ArgSource`.
pub static ARG_SOURCE: UnionDecl = UnionDecl {
    name: "ArgSource",
    key: "$type",
    cases: &[
        case("literal", &[req("value", Slot::Str)]),
        case("fromNode", &[req("node", Slot::Str)]),
    ],
};

/// An invoke node's argument binding: the anonymous `(addr, source)` tuple.
pub static ARG_BINDING: RecordDecl = RecordDecl {
    name: "ArgBinding",
    members: &[
        req("addr", Slot::Str),
        req("source", Slot::Union(&ARG_SOURCE)),
    ],
    strict: false,
};

/// `PipelineNode`.
pub static PIPELINE_NODE: UnionDecl = UnionDecl {
    name: "PipelineNode",
    key: "$type",
    cases: &[
        case(
            "source",
            &[
                req("id", Slot::Str),
                req("dataRef", Slot::Str),
                req("outputType", Slot::Hosted(Hosted::OutputSpace)),
            ],
        ),
        case(
            "invoke",
            &[
                req("id", Slot::Str),
                req("capabilityId", Slot::Str),
                req("outputType", Slot::Hosted(Hosted::OutputSpace)),
                req("args", Slot::List(&Slot::Record(&ARG_BINDING))),
            ],
        ),
    ],
};

/// `CapabilityPipeline`.
pub static CAPABILITY_PIPELINE: RecordDecl = RecordDecl {
    name: "CapabilityPipeline",
    members: &[req("nodes", Slot::List(&Slot::Union(&PIPELINE_NODE)))],
    strict: true,
};

const COMPARED: &[Member] = &[
    req("column", Slot::Str),
    req("type", Slot::Hosted(Hosted::ColumnType)),
    req("value", Slot::Str),
];

const fn compared(tag: &'static str) -> Case {
    Case {
        tag,
        members: COMPARED,
        codec: Some(CaseCodec::Compared),
    }
}

/// `ColumnPredicate`.
pub static COLUMN_PREDICATE: UnionDecl = UnionDecl {
    name: "ColumnPredicate",
    key: "$type",
    cases: &[
        compared("equalTo"),
        compared("greaterThan"),
        compared("atLeast"),
        compared("lessThan"),
        compared("atMost"),
        case(
            "contains",
            &[req("column", Slot::Str), req("text", Slot::Str)],
        ),
        case("isNull", &[req("column", Slot::Str)]),
        case("isNotNull", &[req("column", Slot::Str)]),
    ],
};

const COLUMN_TYPE: Slot = Slot::Hosted(Hosted::ColumnType);

/// `QueryError`, the query seam's refusal.
pub static QUERY_ERROR: UnionDecl = UnionDecl {
    name: "QueryError",
    key: "$type",
    cases: &[
        case("noSuchQuery", &[req("id", Slot::Str), req("known", STRS)]),
        case("duplicateQuery", &[req("id", Slot::Str)]),
        case(
            "unknownParam",
            &[req("name", Slot::Str), req("declared", STRS)],
        ),
        case(
            "paramTypeMismatch",
            &[
                req("name", Slot::Str),
                req("expected", COLUMN_TYPE),
                req("got", COLUMN_TYPE),
            ],
        ),
        case("requiredParamsUnbound", &[req("names", STRS)]),
        case("sourceNotResolved", &[req("ref", Slot::Str)]),
        case(
            "executionFailed",
            &[req("detail", Slot::Str), req("recoverable", STRS)],
        ),
        case("timeout", &[]),
        case("requiredParamsNull", &[req("names", STRS)]),
        case("duplicateParam", &[req("name", Slot::Str)]),
        case(
            "queryPolicyRefused",
            &[
                req("policy", Slot::Str),
                req("reason", Slot::Str),
                req("allowed", STRS),
            ],
        ),
        case("queryApprovalRequired", &[req("policy", Slot::Str)]),
        case(
            "unreadableArgs",
            &[req("error", Slot::Hosted(Hosted::DecodeError))],
        ),
        case(
            "unknownColumn",
            &[req("name", Slot::Str), req("declared", STRS)],
        ),
        case(
            "predicateTypeMismatch",
            &[
                req("column", Slot::Str),
                req("expected", COLUMN_TYPE),
                req("got", COLUMN_TYPE),
            ],
        ),
        case(
            "predicateNotApplicable",
            &[
                req("predicate", Slot::Hosted(Hosted::PredicateTag)),
                req("column", Slot::Str),
                req("columnType", COLUMN_TYPE),
            ],
        ),
        case(
            "illFormedLiteral",
            &[req("column", Slot::Str), req("reason", Slot::Str)],
        ),
        case("duplicateSortColumn", &[req("column", Slot::Str)]),
        case(
            "predicateNotHonoured",
            &[req("predicate", Slot::Union(&COLUMN_PREDICATE))],
        ),
        case("orderNotHonoured", &[req("column", Slot::Str)]),
    ],
};

/// `SortDirection`.
pub static SORT_DIRECTION: EnumDecl = EnumDecl {
    name: "SortDirection",
    values: &["ascending", "descending"],
};

/// `SortKey`.
pub static SORT_KEY: RecordDecl = RecordDecl {
    name: "SortKey",
    members: &[
        req("column", Slot::Str),
        req("direction", Slot::Enum(&SORT_DIRECTION)),
    ],
    strict: true,
};

const UNIT: Slot = Slot::Hosted(Hosted::Unit);

/// `FieldChange`, the schema delta's member change, discriminated by `member`.
pub static FIELD_CHANGE: UnionDecl = UnionDecl {
    name: "FieldChange",
    key: "member",
    cases: &[
        case("unit", &[opt("from", UNIT), opt("to", UNIT)]),
        case("label", &[opt("from", Slot::Str), opt("to", Slot::Str)]),
        case(
            "description",
            &[opt("from", Slot::Str), opt("to", Slot::Str)],
        ),
        case(
            "ext",
            &[
                req("key", Slot::Str),
                opt("from", Slot::Str),
                opt("to", Slot::Str),
            ],
        ),
    ],
};

/// `NestRelation`.
pub static NEST_RELATION: EnumDecl = EnumDecl {
    name: "NestRelation",
    values: &["self", "descendant"],
};

const IDS: Slot = Slot::List(&Slot::Param);

/// `Rejection<'Id>`, the ops rejection envelope: every node id is the type parameter.
pub static REJECTION: UnionDecl = UnionDecl {
    name: "Rejection",
    key: "$type",
    cases: &[
        case(
            "unknownNode",
            &[req("target", Slot::Param), req("addressable", IDS)],
        ),
        case("duplicateId", &[req("id", Slot::Param)]),
        case("cannotRemoveRoot", &[]),
        case(
            "wouldNestUnderSelf",
            &[
                req("target", Slot::Param),
                req("relation", Slot::Enum(&NEST_RELATION)),
            ],
        ),
        case(
            "notAContainer",
            &[req("target", Slot::Param), req("kindTag", Slot::Str)],
        ),
        case(
            "reorderMismatch",
            &[
                req("parent", Slot::Param),
                req("expected", IDS),
                req("got", IDS),
            ],
        ),
        case(
            "rejected",
            &[req("code", Slot::Str), req("message", Slot::Str)],
        ),
        case(
            "keyedPosition",
            &[
                req("target", Slot::Param),
                req("holder", Slot::Param),
                req("slot", Slot::Int),
            ],
        ),
        case(
            "illegalChild",
            &[
                req("child", Slot::Param),
                req("childKind", Slot::Str),
                req("parent", Slot::Param),
                req("parentKind", Slot::Str),
                req("legal", STRS),
            ],
        ),
        case(
            "stillReferenced",
            &[req("target", Slot::Param), req("referrers", IDS)],
        ),
        case(
            "rootIdMismatch",
            &[req("before", Slot::Param), req("after", Slot::Param)],
        ),
    ],
};

/// The seam readers by name, as the conformance vectors name them; a `/strict`
/// suffix selects the strict-members reading of the same root.
pub static READERS: &[(&str, Slot)] = &[
    ("invokeError", Slot::Union(&INVOKE_ERROR)),
    ("declarationFault", Slot::Union(&DECLARATION_FAULT)),
    ("placement", Slot::Union(&PLACEMENT)),
    ("deferred", Slot::Union(&DEFERRED)),
    ("pipeline", Slot::Record(&CAPABILITY_PIPELINE)),
    ("pipelineNode", Slot::Union(&PIPELINE_NODE)),
    ("argSource", Slot::Union(&ARG_SOURCE)),
    ("queryError", Slot::Union(&QUERY_ERROR)),
    ("columnPredicate", Slot::Union(&COLUMN_PREDICATE)),
    ("sortKey", Slot::Record(&SORT_KEY)),
    ("fieldChange", Slot::Union(&FIELD_CHANGE)),
    ("rejection", Slot::Union(&REJECTION)),
    ("decodeError", Slot::Hosted(Hosted::DecodeError)),
];

/// A reader by its vector name (`invokeError`, `placement/strict`, …): the root slot
/// and whether it is the strict reading. `None` for a name no seam declares.
pub fn reader(name: &str) -> Option<(Slot, bool)> {
    let (base, strict) = match name.strip_suffix("/strict") {
        Some(b) => (b, true),
        None => (name, false),
    };
    READERS
        .iter()
        .find(|(n, _)| *n == base)
        .map(|(_, s)| (*s, strict))
}

// ─── The reading ───────────────────────────────────────────────────────────

/// How one document is read: the strict-members reading or not, and the slot a
/// union's type parameter is instantiated at.
pub struct Reading {
    pub strict: bool,
    pub param: Slot,
}

type R<T> = Result<T, DecodeError>;

/// Read `doc` at `root` — twin of the derived `decode<Name>` (or, under
/// `reading.strict`, `decode<Name>Strict`: the members check over the whole
/// document first, then the decode) followed by `encode<Name>`. Answers the
/// re-encoding of what it read, or the refusal at its path.
pub fn read(root: &Slot, doc: &Value, reading: &Reading) -> R<JVal> {
    let at = Cursor::root(doc);
    if reading.strict {
        strict(root, &at)?;
    }
    decode(root, &at, reading)
}

/// Read JSON text at `root` and answer the re-encoding's canonical bytes. The text
/// is parsed under the strict limits; a malformed one is `InvalidJson`.
pub fn read_text(root: &Slot, text: &str, reading: &Reading) -> R<String> {
    let doc = crate::decode::parse(text, &crate::json::JsonLimits::STRICT)?;
    read(root, &doc, reading).map(|v| render_canonical(&v))
}

/// The reference's one sentence of a refusal (`DecodeError.sentence`), for the
/// refusals this module raises itself.
fn sentence(code: DecodeCode, expected: &str, detail: &str) -> String {
    let led = |phrase: &str| {
        if detail.is_empty() {
            format!("{phrase}; expected {expected}")
        } else {
            format!("{phrase}: {detail}; expected {expected}")
        }
    };
    match code {
        DecodeCode::InvalidJson => format!("not valid JSON: {detail}"),
        DecodeCode::MissingField => format!("missing property: {detail}"),
        DecodeCode::WrongKind => format!("expected {expected}, got {detail}"),
        DecodeCode::UnknownTag => format!("unknown {detail}; expected {expected}"),
        DecodeCode::OutOfRange => led("out of range"),
        DecodeCode::UndeclaredMember => led("undeclared member"),
        DecodeCode::LimitExceeded => led("limit exceeded"),
        DecodeCode::NotAdmitted => led("not admitted"),
        DecodeCode::SchemaFault => led("schema fault"),
    }
}

fn fail(at: &Cursor<'_>, code: DecodeCode, expected: &str, detail: &str) -> DecodeError {
    at.fail(code, sentence(code, expected, detail))
}

/// A closed string set, read by value: a non-string is `WrongKind`, a miss
/// `UnknownTag` at the value (`Decoder.oneOf`, and a derived enum's decoder).
fn one_of<'a>(at: &Cursor<'a>, known: &[&'static str], what: &str) -> R<&'static str> {
    let s = at.string()?;
    known.iter().copied().find(|k| *k == s).ok_or_else(|| {
        fail(
            at,
            DecodeCode::UnknownTag,
            &format!("one of {}", quote_all(known)),
            &format!("{what} '{s}'"),
        )
    })
}

fn is_obj(v: &Value) -> bool {
    matches!(v, Value::Obj(_))
}

// ─── The strict-members reading ─────────────────────────────────────────────

/// The strict walk (Phase 550): every declared object is held to the members its
/// declaration names — the first undeclared member, in document order, is
/// `UndeclaredMember` at it — and the walk enters each present member with a
/// declared shape, in declaration order. A non-object, a union object whose
/// discriminator names no case, a hosted slot and a type parameter pass: the
/// decode refuses what is wrong with them.
fn strict(slot: &Slot, at: &Cursor<'_>) -> R<()> {
    match slot {
        Slot::Union(u) => {
            if !is_obj(at.value()) {
                return Ok(());
            }
            let Some(tag) = at.optional(u.key)? else {
                return Ok(());
            };
            let Value::Str(t) = tag.value() else {
                return Ok(());
            };
            let Some(c) = u.cases.iter().find(|c| c.tag == t.as_str()) else {
                return Ok(());
            };
            let mut names: Vec<&str> = vec![u.key];
            names.extend(c.members.iter().map(|m| m.name));
            at.closed(&names)?;
            strict_members(c.members, at)
        }
        Slot::Record(r) if r.strict => {
            if !is_obj(at.value()) {
                return Ok(());
            }
            let names: Vec<&str> = r.members.iter().map(|m| m.name).collect();
            at.closed(&names)?;
            strict_members(r.members, at)
        }
        Slot::List(item) => match at.value() {
            Value::Arr(_) => {
                for c in at.items()? {
                    strict(item, &c)?;
                }
                Ok(())
            }
            _ => Ok(()),
        },
        _ => Ok(()),
    }
}

fn strict_members(members: &[Member], at: &Cursor<'_>) -> R<()> {
    for m in members {
        if let Some(c) = at.optional(m.name)? {
            strict(&m.slot, &c)?;
        }
    }
    Ok(())
}

// ─── The decode ─────────────────────────────────────────────────────────────

fn decode(slot: &Slot, at: &Cursor<'_>, reading: &Reading) -> R<JVal> {
    match slot {
        Slot::Str => Ok(JVal::Str(at.string()?.to_string())),
        Slot::Int => Ok(JVal::Num(f64::from(at.int()?))),
        Slot::Number => Ok(JVal::Num(at.float()?)),
        Slot::Param => {
            let param = reading.param;
            if matches!(param, Slot::Param) {
                // A parameter instantiated at itself has no reader: refuse rather than loop.
                return Err(fail(
                    at,
                    DecodeCode::SchemaFault,
                    "a reader for the type parameter",
                    "the type parameter is unresolved",
                ));
            }
            decode(&param, at, reading)
        }
        Slot::List(item) => {
            let mut out = Vec::new();
            for c in at.items()? {
                out.push(decode(item, &c, reading)?);
            }
            Ok(JVal::Arr(out))
        }
        Slot::Enum(e) => {
            let v = one_of(at, e.values, "case")?;
            Ok(JVal::Str(v.to_string()))
        }
        Slot::Union(u) => decode_union(u, at, reading),
        Slot::Record(r) => {
            if !is_obj(at.value()) {
                return Err(fail(
                    at,
                    DecodeCode::WrongKind,
                    "object",
                    at.value().kind_name(),
                ));
            }
            Ok(JVal::Obj(decode_members(r.members, at, reading)?))
        }
        Slot::Hosted(h) => decode_hosted(*h, at, reading),
    }
}

/// A union: an object, then its discriminator (absent `MissingField`, a non-string
/// `WrongKind`, an unknown tag `UnknownTag`, all at the key), then the case's
/// members in declaration order.
fn decode_union(u: &UnionDecl, at: &Cursor<'_>, reading: &Reading) -> R<JVal> {
    if !is_obj(at.value()) {
        return Err(fail(
            at,
            DecodeCode::WrongKind,
            "object",
            at.value().kind_name(),
        ));
    }
    let known: Vec<&str> = u.cases.iter().map(|c| c.tag).collect();
    let tag = at.tag(u.key, &known)?;
    let Some(c) = u.cases.iter().find(|c| c.tag == tag) else {
        return Err(fail(at, DecodeCode::SchemaFault, "a declared case", tag));
    };
    let mut fields = vec![(u.key.to_string(), JVal::Str(c.tag.to_string()))];
    match c.codec {
        None => fields.extend(decode_members(c.members, at, reading)?),
        Some(CaseCodec::Compared) => fields.extend(compared_of(at)?),
    }
    Ok(JVal::Obj(fields))
}

fn decode_members(
    members: &[Member],
    at: &Cursor<'_>,
    reading: &Reading,
) -> R<Vec<(String, JVal)>> {
    let mut out = Vec::new();
    for m in members {
        let c = if m.optional {
            match at.optional(m.name)? {
                Some(c) => c,
                None => continue,
            }
        } else {
            at.field(m.name)?
        };
        out.push((m.name.to_string(), decode(&m.slot, &c, reading)?));
    }
    Ok(out)
}

fn decode_hosted(h: Hosted, at: &Cursor<'_>, reading: &Reading) -> R<JVal> {
    match h {
        Hosted::Space => decode_union(&VALUE_SPACE, at, reading),
        Hosted::OutputSpace => {
            let space = decode_union(&VALUE_SPACE, at, reading)?;
            match well_formed(&space) {
                Ok(()) => Ok(space),
                Err(detail) => Err(fail(
                    at,
                    DecodeCode::OutOfRange,
                    "a well-formed value space",
                    &detail,
                )),
            }
        }
        Hosted::ColumnType => Ok(JVal::Str(
            one_of(at, &ColumnType::TAGS, "value")?.to_string(),
        )),
        Hosted::PredicateTag => Ok(JVal::Str(one_of(at, PREDICATE_TAGS, "value")?.to_string())),
        Hosted::DecodeError => decode_error_form(at),
        Hosted::Unit => {
            let s = at.string()?;
            match unit::parse(s) {
                Ok(u) => Ok(JVal::Str(unit::render(&u))),
                Err(_) => Err(fail(
                    at,
                    DecodeCode::OutOfRange,
                    "a unit the kit parses",
                    &format!("'{s}' is not a unit the kit parses"),
                )),
            }
        }
    }
}

// ─── Hosted: value spaces ───────────────────────────────────────────────────

fn num(v: &JVal, key: &str) -> f64 {
    match v.field(key) {
        Some(JVal::Num(n)) => *n,
        _ => 0.0,
    }
}

/// Twin of `Space.wellFormed`, over a space as `SpaceCodec` re-encodes it: a
/// non-finite float bound first, then an empty range, a `StringLen` with a negative
/// `max`, or an `Enum` with no member. Answers the detail of the refusal.
fn well_formed(space: &JVal) -> Result<(), String> {
    let tag = match space.field("$type") {
        Some(JVal::Str(t)) => t.as_str(),
        _ => return Ok(()),
    };
    let (lo, hi) = (num(space, "min"), num(space, "max"));
    match tag {
        "floatRange" if !(lo.is_finite() && hi.is_finite()) => {
            Err("value-space bound is not a finite number".to_string())
        }
        "intRange" | "floatRange" if lo > hi => {
            Err(format!("empty value-space: {}", render_canonical(space)))
        }
        "stringLen" if lo > hi || hi < 0.0 => {
            Err(format!("empty value-space: {}", render_canonical(space)))
        }
        "enum" if matches!(space.field("values"), Some(JVal::Arr(xs)) if xs.is_empty()) => {
            Err("empty value-space: a member of an empty set, so no value is accepted".to_string())
        }
        _ => Ok(()),
    }
}

// ─── Hosted: the decode error form ──────────────────────────────────────────

const DECODE_CODES: &[&str] = &[
    "InvalidJson",
    "MissingField",
    "WrongKind",
    "UnknownTag",
    "OutOfRange",
    "UndeclaredMember",
    "LimitExceeded",
    "NotAdmitted",
    "SchemaFault",
];

/// Twin of `DecodeError.decoder` then `DecodeError.toJson`: `code` one of the
/// decode codes by name, `path` an array of member names and item indices,
/// `expected` and `message` strings, read in that order; the message is kept
/// verbatim.
fn decode_error_form(at: &Cursor<'_>) -> R<JVal> {
    if !is_obj(at.value()) {
        return Err(fail(
            at,
            DecodeCode::WrongKind,
            "object",
            at.value().kind_name(),
        ));
    }
    let code = one_of(&at.field("code")?, DECODE_CODES, "value")?;
    let path_at = at.field("path")?;
    let path = match path_at.value() {
        Value::Arr(items) => items
            .iter()
            .map(|v| match v {
                Value::Str(k) => Some(JVal::Str(k.clone())),
                Value::Int(i) => Some(JVal::Num(f64::from(*i))),
                _ => None,
            })
            .collect::<Option<Vec<JVal>>>(),
        _ => None,
    };
    let Some(path) = path else {
        return Err(fail(
            &path_at,
            DecodeCode::WrongKind,
            "an array of member names and item indices",
            "not a decode path",
        ));
    };
    let expected = at.field("expected")?.string()?.to_string();
    let message = at.field("message")?.string()?.to_string();
    Ok(JVal::Obj(vec![
        ("code".to_string(), JVal::Str(code.to_string())),
        ("path".to_string(), JVal::Arr(path)),
        ("expected".to_string(), JVal::Str(expected)),
        ("message".to_string(), JVal::Str(message)),
    ]))
}

// ─── Hosted: column types, predicate tags and comparison literals ──────────

/// The predicate wire tags, in case order (twin of `QueryWire.predicateTags`).
pub const PREDICATE_TAGS: &[&str] = &[
    "equalTo",
    "greaterThan",
    "atLeast",
    "lessThan",
    "atMost",
    "contains",
    "isNull",
    "isNotNull",
];

/// Twin of `QueryWire.comparedOf` then `comparedMembers`: `column`, then `type`,
/// then `value` read as one present cell of that type; a value the type cannot carry
/// is `OutOfRange` at `value`. The re-encoding names the type the CELL carries (an
/// instant at the coarsest unit that holds it) and the value as the column codec
/// writes it.
fn compared_of(at: &Cursor<'_>) -> R<Vec<(String, JVal)>> {
    let column = at.field("column")?.string()?.to_string();
    let ty = one_of(&at.field("type")?, &ColumnType::TAGS, "value")?;
    let value_at = at.field("value")?;
    let Some((cell_ty, written)) = read_literal(ty, value_at.value()) else {
        return Err(fail(
            &value_at,
            DecodeCode::OutOfRange,
            &format!("a {ty} value"),
            &format!(
                "the filter on column '{column}' compares with a value a {ty} column cannot carry"
            ),
        ));
    };
    Ok(vec![
        ("column".to_string(), JVal::Str(column)),
        ("type".to_string(), JVal::Str(cell_ty.to_string())),
        ("value".to_string(), written),
    ])
}

/// One present cell of the column type `ty`, read as the column codec reads it and
/// written as it writes the cell back (twin of `ColumnCodec.readCell` then
/// `writeCell` at the cell's own type): the cell's type tag and its JSON value, or
/// `None` where the value is not one a column of that type carries.
pub fn read_literal(ty: &str, v: &Value) -> Option<(&'static str, JVal)> {
    if matches!(v, Value::Float(f) if !f.is_finite()) {
        return None;
    }
    let ty = ColumnType::of_tag(ty)?;
    let written = ty.round_trip_cell(v)?;
    let cell_ty = match (ty, &written) {
        (ColumnType::Timestamp(_), JVal::Str(text)) => {
            ColumnType::Timestamp(temporal::unit_of(text))
        }
        _ => ty,
    };
    Some((cell_ty.tag(), written))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(ty: &str, text: &str) -> Option<(&'static str, String)> {
        let v = crate::decode::parse(text, &crate::json::JsonLimits::STRICT).unwrap();
        read_literal(ty, &v).map(|(t, j)| (t, render_canonical(&j)))
    }

    #[test]
    fn a_literal_is_read_and_written_as_the_column_codec_does() {
        assert_eq!(lit("float", "1"), Some(("float", "1".to_string())));
        assert_eq!(lit("int", "1.5"), None);
        assert_eq!(
            lit("decimal", "\"12.50\""),
            Some(("decimal", "\"12.5\"".to_string()))
        );
        assert_eq!(lit("date", "\"2026-02-29\""), None);
        assert_eq!(
            lit("timestamp_ms", "\"2026-10-20T01:02:03Z\""),
            Some(("timestamp", "\"2026-10-20T01:02:03Z\"".to_string()))
        );
        assert_eq!(
            lit("timestamp_ms", "1500"),
            Some(("timestamp_ms", "\"1970-01-01T00:00:01.5Z\"".to_string()))
        );
        assert_eq!(
            lit("timestamp_ms", "-1"),
            Some(("timestamp_ms", "\"1969-12-31T23:59:59.999Z\"".to_string()))
        );
        assert_eq!(lit("timestamp", "\"2026-10-20T01:02:03.5Z\""), None);
    }
}
