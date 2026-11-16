//! The invocable capability: a typed, registrable declaration of compute a host
//! runs on a caller's behalf and of what it answers, the default-deny argument
//! check that runs before any body, the effect-identity key a non-deterministic
//! invocation is journalled and replayed under, the identity a grant is bound
//! to, the canonical declaration codec, the id-ordered registry an agent
//! enumerates with the gates it decides under, and the type check of a pipeline
//! of calls.
//!
//! The twin of the reference's `Capability`, `CapabilityCodec`,
//! `CapabilityRegistry`, `Policy` and `CapabilityPipeline.typeCheck`
//! (`Fuaran.Core.Function`), together with the pieces of the signature surface
//! they stand on — the value space (`Space`, `SpaceCodec`), the two-axis effect
//! class (`Effect`, `EffectCodec`), the signature entry and its well-formedness
//! check (`Signature.validate`). Certified against Core's
//! `laws/capability-laws.json` and `laws/capability-document-laws.json`
//! (`tests/capability.rs`) and `laws/capability-gate-laws.json`
//! (`tests/capability_gate.rs`).
//!
//! The wire carries the declaration and a typed invocation, never the body:
//! the core validates, keys, decides and round-trips. Running the body, and
//! routing it by its [`Placement`], is the host's; [`CapabilityRegistry::dispatch`]
//! hands an admitted invocation to the host's performer and runs nothing else.

use crate::decode::{self, Cursor, DecodeCode, DecodeError, quote_all};
use crate::hash::{FOLD_SEP, FieldBody, keyed_by_preimage, write_canonical_field};
use crate::json::{JVal, JsonLimits, ordinal_cmp, render_canonical};
use crate::number::write_finite_double;
use crate::prelude::*;
use crate::seam::{self, Reading, Slot};
use alloc::sync::Arc;
use core::cmp::Ordering;
use core::fmt::{self, Write};

// ─── The effect class ────────────────────────────────────────────────────────

/// Axis 1 of the effect class — does the body touch the world. Twin of `HostEffect`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostEffect {
    /// Touches nothing outside its inputs; wire tag `pure`.
    Pure,
    /// Reads state outside its inputs but changes none; wire tag `readsHost`.
    ReadsHost,
    /// Changes state outside its inputs; wire tag `writesHost`.
    WritesHost,
}

impl HostEffect {
    /// The wire spelling. Twin of `EffectCodec.hostTag`.
    pub fn tag(self) -> &'static str {
        match self {
            HostEffect::Pure => "pure",
            HostEffect::ReadsHost => "readsHost",
            HostEffect::WritesHost => "writesHost",
        }
    }

    const ALL: [HostEffect; 3] = [
        HostEffect::Pure,
        HostEffect::ReadsHost,
        HostEffect::WritesHost,
    ];
}

/// One atomic source of non-determinism. The declaration order is the canonical
/// order every label renders a set in. Twin of `DeterminismFactor`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum DeterminismFactor {
    /// Reads the wall clock; `clock` on the wire.
    Clock,
    /// Reads a random source; `random` on the wire.
    Random,
    /// Reads the network; `network` on the wire.
    Network,
}

impl DeterminismFactor {
    /// The factors in canonical order.
    pub const CANONICAL: [DeterminismFactor; 3] = [
        DeterminismFactor::Clock,
        DeterminismFactor::Random,
        DeterminismFactor::Network,
    ];

    /// The factor's wire name.
    pub fn name(self) -> &'static str {
        match self {
            DeterminismFactor::Clock => "clock",
            DeterminismFactor::Random => "random",
            DeterminismFactor::Network => "network",
        }
    }

    fn bit(self) -> u8 {
        match self {
            DeterminismFactor::Clock => 1,
            DeterminismFactor::Random => 2,
            DeterminismFactor::Network => 4,
        }
    }
}

/// The determinism axis: the SET of factors a body reads, joined by union; the
/// empty set is deterministic. Twin of `DeterminismSource` (`Set<DeterminismFactor>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DeterminismSet(u8);

impl DeterminismSet {
    /// The empty set: reproducible from inputs. Twin of `Effect.deterministic`.
    pub const DETERMINISTIC: DeterminismSet = DeterminismSet(0);

    /// The set of the given factors.
    pub fn of(factors: &[DeterminismFactor]) -> DeterminismSet {
        DeterminismSet(factors.iter().fold(0, |acc, f| acc | f.bit()))
    }

    /// Does the set hold `f`.
    pub fn contains(self, f: DeterminismFactor) -> bool {
        self.0 & f.bit() != 0
    }

    /// The set with `f` added.
    pub fn with(self, f: DeterminismFactor) -> DeterminismSet {
        DeterminismSet(self.0 | f.bit())
    }

    /// The union. Twin of the determinism axis of `Effect.join`.
    pub fn union(self, other: DeterminismSet) -> DeterminismSet {
        DeterminismSet(self.0 | other.0)
    }

    /// Is every factor of `self` in `other`.
    pub fn is_subset(self, other: DeterminismSet) -> bool {
        self.0 & !other.0 == 0
    }

    /// Is the set empty.
    pub fn is_deterministic(self) -> bool {
        self.0 == 0
    }

    /// The canonical wire label: `deterministic` for the empty set, otherwise the
    /// members' names in canonical order joined by `+`. Twin of `Effect.determinismTag`.
    pub fn tag(self) -> String {
        let names: Vec<&str> = DeterminismFactor::CANONICAL
            .iter()
            .filter(|f| self.contains(**f))
            .map(|f| f.name())
            .collect();
        if names.is_empty() {
            "deterministic".to_string()
        } else {
            names.join("+")
        }
    }

    /// The inverse of [`DeterminismSet::tag`] on exactly the canonical labels: a
    /// reordered, repeated, empty or unknown member is `None`, so a set has one
    /// wire spelling. Twin of `Effect.tryDeterminismOfTag`.
    pub fn of_tag(tag: &str) -> Option<DeterminismSet> {
        let parsed = if tag == "deterministic" {
            DeterminismSet::DETERMINISTIC
        } else {
            let mut set = DeterminismSet::DETERMINISTIC;
            for name in tag.split('+') {
                let f = DeterminismFactor::CANONICAL
                    .iter()
                    .find(|f| f.name() == name)?;
                set = set.with(*f);
            }
            set
        };
        (parsed.tag() == tag).then_some(parsed)
    }
}

/// The mandatory two-axis effect signature. Twin of `EffectClass`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectClass {
    /// How far the body reaches into the host.
    pub host: HostEffect,
    /// Every non-deterministic source the body reads.
    pub determinism: DeterminismSet,
}

// ─── The value space ─────────────────────────────────────────────────────────

/// The value domain a hole ranges over. Twin of `ValueSpace`.
#[derive(Debug, Clone, PartialEq)]
pub enum ValueSpace {
    /// An integer in `[lo, hi]`, written with no `+`, white space or fraction.
    IntRange(i32, i32),
    /// A finite decimal number in `[lo, hi]`.
    FloatRange(f64, f64),
    /// A string whose length in UTF-16 code units is in `[lo, hi]`.
    StringLen(i32, i32),
    /// Exactly one of the listed members, compared ordinally.
    Enum(Vec<String>),
    /// Any string.
    AnyString,
    /// A `"kind"`-tagged wire document, of the constrained kind when one is given.
    SlotTree(Option<String>),
    /// A well-formed JSON document, of the named type when one is given: the
    /// name is carried and compared, never interpreted. Unbounded, admitted by no
    /// scalar space and by no tree space (DECISIONS.md D13).
    Document(Option<String>),
}

/// Why a value space is not well-formed. Twin of `SpaceFault`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceFault {
    /// The space admits no value.
    Empty,
    /// A `FloatRange` bound is NaN or infinite.
    NonFinite,
}

/// The largest count a repeat hole may range over. Twin of `Space.maxRepeatCount`.
pub const MAX_REPEAT_COUNT: i32 = 1_000_000;

/// The kind tag of a tree argument: `Some` when the string is a well-formed wire
/// document whose top level is an object with a string `"kind"`. Twin of `Space.slotKindOf`.
pub fn slot_kind_of(s: &str) -> Option<String> {
    match decode::parse(s, &JsonLimits::STRICT) {
        Ok(v @ decode::Value::Obj(_)) => {
            let c = Cursor::root(&v);
            c.field("kind")
                .ok()
                .and_then(|k| k.string().ok().map(String::from))
        }
        _ => None,
    }
}

/// A document argument as the strict reader reads it: `Some` when the string is
/// one JSON document holding no `null`. The reader refuses a repeated member, an
/// ill-formed string and a number no finite double spells, so what is read has a
/// canonical rendering of its own. Read without recursion (D10); the `null`
/// check walks the value with an explicit stack.
fn read_document(s: &str) -> Option<JVal> {
    let v = crate::json::parse(s).ok()?;
    let mut pending: Vec<&JVal> = vec![&v];
    while let Some(x) = pending.pop() {
        match x {
            JVal::Null => return None,
            JVal::Arr(items) => pending.extend(items),
            JVal::Obj(members) => pending.extend(members.iter().map(|(_, m)| m)),
            JVal::Bool(_) | JVal::Num(_) | JVal::Str(_) => {}
        }
    }
    Some(v)
}

/// The one spelling of a document argument: `Some` its cross-host canonical
/// text (members sorted, no white space, the canonical number form and string
/// escape) when the string is a document, `None` otherwise. Twin of
/// `Space.documentOf` (DECISIONS.md D13).
pub fn document_of(s: &str) -> Option<String> {
    read_document(s).map(|v| render_canonical(&v))
}

/// An integer argument as the seam reads it: an optional `-` and decimal digits,
/// within `i32`; leading zeros are read. Twin of `Space.readInt` (over `Json.readInt32`).
pub fn read_int(s: &str) -> Option<i32> {
    let digits = s.strip_prefix('-').unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<i32>().ok()
}

/// A number argument as the seam reads it: `-?digits(.digits)?([eE][+-]?digits)?`,
/// finite. Twin of `Space.readFloat`.
pub fn read_float(s: &str) -> Option<f64> {
    let b = s.as_bytes();
    let n = b.len();
    let digits_from = |mut i: usize| {
        let from = i;
        while i < n && b[i].is_ascii_digit() {
            i += 1;
        }
        (i, i > from)
    };
    let (mut i, mut shaped) = digits_from(usize::from(n > 0 && b[0] == b'-'));
    if shaped && i < n && b[i] == b'.' {
        (i, shaped) = digits_from(i + 1);
    }
    if shaped && i < n && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < n && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        (i, shaped) = digits_from(i);
    }
    if !shaped || i != n {
        return None;
    }
    s.parse::<f64>().ok().filter(|v| v.is_finite())
}

fn utf16_len(s: &str) -> i64 {
    s.encode_utf16().count() as i64
}

/// The canonical spelling of a value in a space that respells it: an integer
/// in its shortest decimal form, a number in the canonical number form. Written
/// through `core::fmt`, so a value is spelled where it is written and never
/// built first (DECISIONS.md D12).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Respelled {
    Int(i32),
    Num(f64),
}

impl core::fmt::Display for Respelled {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Respelled::Int(v) => write!(f, "{v}"),
            Respelled::Num(v) => write_finite_double(*v, f),
        }
    }
}

impl ValueSpace {
    /// The value an integer or number space reads `s` as, where `s` lies in it:
    /// the one place a numeric value is read, checked against its bounds and
    /// spelled. `None` for every other space, and for a value outside the space.
    fn respelled(&self, s: &str) -> Option<Respelled> {
        match self {
            ValueSpace::IntRange(lo, hi) => read_int(s)
                .filter(|v| v >= lo && v <= hi)
                .map(Respelled::Int),
            ValueSpace::FloatRange(lo, hi) => read_float(s)
                .filter(|v| v >= lo && v <= hi)
                .map(Respelled::Num),
            _ => None,
        }
    }

    /// The one spelling a value in the space is handed on as, or `None` where the
    /// string is not in the space: an integer in its shortest decimal form, a
    /// number in the canonical number form, a document in its cross-host
    /// canonical text ([`document_of`]), every other value as sent. Twin of
    /// `Space.canonical`.
    pub fn canonical(&self, s: &str) -> Option<String> {
        match self {
            ValueSpace::IntRange(..) | ValueSpace::FloatRange(..) => {
                self.respelled(s).map(|r| r.to_string())
            }
            ValueSpace::Document(_) => document_of(s),
            _ => self.validate(s).then(|| s.to_string()),
        }
    }

    /// Is the value in the space — exactly when it has a canonical spelling there
    /// — decided on the value, without spelling it: a number read and compared
    /// with the bounds, a length counted, a member compared. A tree and a document
    /// are the spaces whose membership builds anything: each is read by the strict
    /// reader (D12, D13), and a document's text is not rendered. Twin of
    /// `Space.validate`.
    pub fn validate(&self, s: &str) -> bool {
        match self {
            ValueSpace::IntRange(..) | ValueSpace::FloatRange(..) => self.respelled(s).is_some(),
            ValueSpace::StringLen(lo, hi) => {
                let len = utf16_len(s);
                len >= i64::from(*lo) && len <= i64::from(*hi)
            }
            ValueSpace::Enum(xs) => xs.iter().any(|x| x == s),
            ValueSpace::AnyString => true,
            ValueSpace::SlotTree(c) => match (slot_kind_of(s), c) {
                (None, _) => false,
                (Some(_), None) => true,
                (Some(k), Some(c)) => &k == c,
            },
            ValueSpace::Document(_) => read_document(s).is_some(),
        }
    }

    /// Is this a count space a repeat may range over: an `IntRange` with
    /// `0 <= lo <= hi <= MAX_REPEAT_COUNT`. Twin of `Space.isCount`.
    pub fn is_count(&self) -> bool {
        matches!(self, ValueSpace::IntRange(lo, hi) if 0 <= *lo && lo <= hi && *hi <= MAX_REPEAT_COUNT)
    }

    /// Does the space admit a value, and can every codec write it back. Twin of
    /// `Space.wellFormed`.
    pub fn well_formed(&self) -> Result<(), SpaceFault> {
        match self {
            ValueSpace::FloatRange(lo, hi) if !(lo.is_finite() && hi.is_finite()) => {
                Err(SpaceFault::NonFinite)
            }
            ValueSpace::IntRange(lo, hi) if lo > hi => Err(SpaceFault::Empty),
            ValueSpace::FloatRange(lo, hi) if lo > hi => Err(SpaceFault::Empty),
            ValueSpace::StringLen(lo, hi) if lo > hi || *hi < 0 => Err(SpaceFault::Empty),
            ValueSpace::Enum(xs) if xs.is_empty() => Err(SpaceFault::Empty),
            _ => Ok(()),
        }
    }

    /// The space in words, for a refusal a model reads. Twin of `Space.describe`.
    pub fn describe(&self) -> String {
        match self {
            ValueSpace::IntRange(lo, hi) => format!("an integer from {lo} to {hi}"),
            ValueSpace::FloatRange(lo, hi) => format!(
                "a number from {} to {}",
                crate::number::format_number(*lo),
                crate::number::format_number(*hi)
            ),
            ValueSpace::StringLen(lo, hi) => format!("a string of {lo} to {hi} characters"),
            ValueSpace::Enum(xs) if xs.is_empty() => {
                "a member of an empty set, so no value is accepted".to_string()
            }
            ValueSpace::Enum(xs) => format!("one of {}", quote_all(xs)),
            ValueSpace::AnyString => "any string".to_string(),
            ValueSpace::SlotTree(None) => "a tree: a JSON object with a \"kind\"".to_string(),
            ValueSpace::SlotTree(Some(k)) => {
                format!("a tree of kind '{k}': a JSON object whose \"kind\" is '{k}'")
            }
            ValueSpace::Document(None) => "a document: well-formed JSON".to_string(),
            ValueSpace::Document(Some(t)) => {
                format!("a document of type '{t}': well-formed JSON")
            }
        }
    }

    /// Does this space (the one required) admit every value `available` can
    /// hold: the bounds compared, an integer range read as numbers where a number
    /// is required, a string length measured in UTF-16 code units; the trees and
    /// the documents each a family of their own, an unconstrained one admitting
    /// every member of its family and a constrained one only the same name. The
    /// one space relation a pipeline's edges and a call's declared output are
    /// checked by. Twin of `Space.subsumes`.
    pub fn subsumes(&self, available: &ValueSpace) -> bool {
        use ValueSpace::*;
        match (self, available) {
            (IntRange(rl, rh), IntRange(al, ah)) => rl <= al && ah <= rh,
            (FloatRange(rl, rh), FloatRange(al, ah)) => rl <= al && ah <= rh,
            (FloatRange(rl, rh), IntRange(al, ah)) => {
                *rl <= f64::from(*al) && f64::from(*ah) <= *rh
            }
            (StringLen(rl, rh), StringLen(al, ah)) => rl <= al && ah <= rh,
            (StringLen(rl, rh), Enum(xs)) => xs.iter().all(|x| {
                let len = utf16_len(x);
                len >= i64::from(*rl) && len <= i64::from(*rh)
            }),
            (Enum(rs), Enum(xs)) => xs.iter().all(|x| rs.contains(x)),
            (AnyString, IntRange(..) | FloatRange(..) | StringLen(..) | Enum(_) | AnyString) => {
                true
            }
            (SlotTree(None), SlotTree(_)) => true,
            (SlotTree(Some(rk)), SlotTree(Some(ak))) => rk == ak,
            (Document(None), Document(_)) => true,
            (Document(Some(rt)), Document(Some(at))) => rt == at,
            _ => false,
        }
    }
}

// ─── The signature ───────────────────────────────────────────────────────────

/// What a hole accepts. Twin of `HoleKind`.
#[derive(Debug, Clone, PartialEq)]
pub enum HoleKind {
    /// A scalar value in the space; wire tag `value`.
    Value(ValueSpace),
    /// A subtree, of the constrained kind when one is given; wire tag `slot`.
    Slot(Option<String>),
    /// A repeated subtree, its count in the space; wire tag `repeat`.
    Repeat(ValueSpace),
    /// A behaviour bound by the host, under the effect ceiling; wire tag `action`.
    Action(EffectClass),
}

impl HoleKind {
    /// The wire tag. Twin of `HoleKind.tag`.
    pub fn tag(&self) -> &'static str {
        match self {
            HoleKind::Value(_) => "value",
            HoleKind::Slot(_) => "slot",
            HoleKind::Repeat(_) => "repeat",
            HoleKind::Action(_) => "action",
        }
    }
}

/// One hole of a signature. Twin of `SigEntry`.
#[derive(Debug, Clone, PartialEq)]
pub struct SigEntry {
    /// The hole's absolute address; the key an argument is bound by.
    pub addr: String,
    /// The display label; never used to bind.
    pub name: String,
    /// What the hole accepts.
    pub kind: HoleKind,
    /// Must an invocation bind it.
    pub required: bool,
}

impl SigEntry {
    /// The space an argument must lie in: a value or repeat hole's space, the
    /// `SlotTree` of a slot's constraint, `None` for an action hole. Twin of `SigEntry.Space`.
    pub fn space(&self) -> Option<ValueSpace> {
        match &self.kind {
            HoleKind::Value(s) | HoleKind::Repeat(s) => Some(s.clone()),
            HoleKind::Slot(c) => Some(ValueSpace::SlotTree(c.clone())),
            HoleKind::Action(_) => None,
        }
    }

    /// Is the entry total: every entry but a repeat over a non-count space. Twin
    /// of `Function.isTotalEntry`.
    pub fn is_total(&self) -> bool {
        match &self.kind {
            HoleKind::Repeat(s) => s.is_count(),
            _ => true,
        }
    }
}

/// A derived signature: its holes, in declaration order, and its declared effect.
/// Twin of `Signature`.
#[derive(Debug, Clone, PartialEq)]
pub struct Signature {
    /// The artifact name.
    pub name: String,
    /// One entry per hole.
    pub holes: Vec<SigEntry>,
    /// The declared effect.
    pub effect: EffectClass,
}

/// Why a declaration is refused at the admission gate. Twin of `DeclarationFault`.
#[derive(Debug, Clone, PartialEq)]
pub enum DeclarationFault {
    /// The hole's space admits no value.
    EmptySpace { addr: String, space: ValueSpace },
    /// The hole has a bound that is not a finite number.
    NonFiniteBound { addr: String },
    /// A second hole at an address an earlier one holds.
    DuplicateHoleAddr { addr: String },
    /// A slot with a hole beneath it (raised by the witness-level check only).
    HoleUnderSlot { node: String },
    /// The declared result ranges over a space that admits no value.
    EmptyResult { space: ValueSpace },
    /// The declared result has a bound that is not a finite number; the fault
    /// carries nothing, because no JSON number spells the space.
    NonFiniteResult,
}

impl DeclarationFault {
    /// The fault in words. Twin of `DeclarationFault.describe`.
    pub fn describe(&self) -> String {
        match self {
            DeclarationFault::EmptySpace { addr, space } => format!(
                "hole '{addr}' ranges over an empty space: {}",
                space.describe()
            ),
            DeclarationFault::NonFiniteBound { addr } => {
                format!("hole '{addr}' has a bound that is not a finite number")
            }
            DeclarationFault::DuplicateHoleAddr { addr } => {
                format!("two holes share the address '{addr}'")
            }
            DeclarationFault::HoleUnderSlot { node } => {
                format!("the slot declared at node '{node}' has a hole beneath it")
            }
            DeclarationFault::EmptyResult { space } => format!(
                "the declared result ranges over an empty space: {}",
                space.describe()
            ),
            DeclarationFault::NonFiniteResult => {
                "the declared result has a bound that is not a finite number".to_string()
            }
        }
    }
}

impl Signature {
    /// The first reason the signature is not a well-formed declaration, in
    /// declaration order: a repeated address, or a space that is empty or has a
    /// non-finite bound. Twin of `Signature.validate`.
    pub fn validate(&self) -> Result<(), DeclarationFault> {
        let mut seen: Vec<&str> = Vec::new();
        for e in &self.holes {
            if seen.contains(&e.addr.as_str()) {
                return Err(DeclarationFault::DuplicateHoleAddr {
                    addr: e.addr.clone(),
                });
            }
            if let Some(sp) = e.space() {
                match sp.well_formed() {
                    Err(SpaceFault::Empty) => {
                        return Err(DeclarationFault::EmptySpace {
                            addr: e.addr.clone(),
                            space: sp,
                        });
                    }
                    Err(SpaceFault::NonFinite) => {
                        return Err(DeclarationFault::NonFiniteBound {
                            addr: e.addr.clone(),
                        });
                    }
                    Ok(()) => {}
                }
            }
            seen.push(&e.addr);
        }
        Ok(())
    }

    /// Is every entry total. Twin of `Function.isTotal`.
    pub fn is_total(&self) -> bool {
        self.holes.iter().all(SigEntry::is_total)
    }
}

// ─── The capability ──────────────────────────────────────────────────────────

/// The kind of client code island a body runs in. Twin of `IslandKind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IslandKind {
    /// Python in the browser; `pyodide`.
    Pyodide,
    /// F# compiled to JavaScript; `fable`.
    Fable,
    /// Plain JavaScript; `js`.
    Js,
}

impl IslandKind {
    /// The wire tag.
    pub fn tag(self) -> &'static str {
        match self {
            IslandKind::Pyodide => "pyodide",
            IslandKind::Fable => "fable",
            IslandKind::Js => "js",
        }
    }

    const ALL: [IslandKind; 3] = [IslandKind::Pyodide, IslandKind::Fable, IslandKind::Js];
}

/// Where the host runs a capability's body; carried and round-tripped, never
/// acted on, by the core. Twin of `Placement`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// When the app is built; `buildTime`.
    BuildTime,
    /// On a server; `server`.
    Server,
    /// In the client, without a code island; `clientDeclarative`.
    ClientDeclarative,
    /// In the client, inside a code island; `clientIsland`.
    ClientIsland(IslandKind),
    /// From a result computed ahead of time; `precomputed`.
    Precomputed,
}

impl Placement {
    /// The `$type` tag.
    pub fn tag(self) -> &'static str {
        match self {
            Placement::BuildTime => "buildTime",
            Placement::Server => "server",
            Placement::ClientDeclarative => "clientDeclarative",
            Placement::ClientIsland(_) => "clientIsland",
            Placement::Precomputed => "precomputed",
        }
    }
}

/// What a capability answers: a value in a space, or nothing. It is stated in
/// the vocabulary a hole speaks, so a result that is a tree of a kind is
/// `Value(SlotTree(Some(kind)))`. "Returns nothing" has this one spelling, and
/// on the wire one document, `{"$type":"nothing"}`. Twin of `Returns`.
#[derive(Debug, Clone, PartialEq)]
pub enum Returns {
    /// The capability answers no value: a call made for its effect.
    Nothing,
    /// The capability answers a value in this space, held to the well-formedness
    /// a hole's space is held to ([`DeclarationFault::EmptyResult`],
    /// [`DeclarationFault::NonFiniteResult`]).
    Value(ValueSpace),
}

impl Returns {
    /// Why the declared result is not well-formed; `Nothing` always is. Twin of
    /// `Capability.resultFault`.
    fn fault(&self) -> Option<DeclarationFault> {
        match self {
            Returns::Nothing => None,
            Returns::Value(space) => match space.well_formed() {
                Ok(()) => None,
                Err(SpaceFault::Empty) => Some(DeclarationFault::EmptyResult {
                    space: space.clone(),
                }),
                Err(SpaceFault::NonFinite) => Some(DeclarationFault::NonFiniteResult),
            },
        }
    }
}

/// A registrable, invocable capability: its registry id, its signature, what it
/// answers and where its body runs. Twin of `Capability` (built by
/// `Capability.create`).
#[derive(Debug, Clone, PartialEq)]
pub struct Capability {
    /// The registry key.
    pub id: String,
    /// The arguments it takes and the effect it declares.
    pub signature: Signature,
    /// What it answers — the one statement of it: a pipeline call's declared
    /// output is checked against it.
    pub result: Returns,
    /// Where its body runs.
    pub placement: Placement,
}

/// Why an invocation, or a registration, was refused. Twin of `InvokeError`.
#[derive(Debug, Clone, PartialEq)]
pub enum InvokeError {
    /// Dispatch named an id the registry does not hold; `known` lists every held id.
    NoSuchCapability { id: String, known: Vec<String> },
    /// Registration under an id the registry already holds.
    DuplicateCapability { id: String },
    /// An argument addresses no hole; `declared` lists every hole address.
    UnknownArg { addr: String, declared: Vec<String> },
    /// An argument's value is not in its hole's space.
    ArgOutOfSpace {
        addr: String,
        space: ValueSpace,
        got: String,
    },
    /// These required holes received no argument, in declaration order.
    RequiredArgsUnbound { addrs: Vec<String> },
    /// The hole cannot take a caller's value: an action hole, or a slot sent no tree.
    UninvocableArg { addr: String },
    /// The body ran and answered a failure.
    BodyFailed { reason: String },
    /// The signature is not total; the named repeat holes count over no count space.
    NonTotalCapability { id: String, addrs: Vec<String> },
    /// The signature is not a well-formed declaration.
    IllFormedCapability { id: String, fault: DeclarationFault },
    /// The invocation binds this address twice.
    DuplicateArg { addr: String },
    /// A registry gate denied the invocation: the deciding gate, the denial's
    /// message and the alternatives it names, each shared with the gate that
    /// made it rather than copied (DECISIONS.md D12). No grant lifts it.
    PolicyRefused {
        policy: Arc<str>,
        reason: Arc<str>,
        allowed: Arc<[Arc<str>]>,
    },
    /// A registry gate requires an approval first, and no grant presented was
    /// for exactly this invocation; the gate's name is shared with the gate.
    ApprovalRequired { policy: Arc<str> },
}

impl InvokeError {
    /// One sentence a model can act on, naming the failure and what would be
    /// accepted. Twin of `InvokeError.describe`.
    pub fn describe(&self) -> String {
        match self {
            InvokeError::NoSuchCapability { id, known } if known.is_empty() => format!(
                "Refused: there is no tool '{id}' you may call. There are no tools you may call."
            ),
            InvokeError::NoSuchCapability { id, known } => format!(
                "Refused: there is no tool '{id}' you may call. The tools you may call are {}.",
                quote_all(known)
            ),
            InvokeError::DuplicateCapability { id } => {
                format!("Refused: the tool '{id}' is registered twice.")
            }
            InvokeError::UnknownArg { addr, declared } if declared.is_empty() => {
                format!("Refused: '{addr}' is not an argument of this tool. It takes no arguments.")
            }
            InvokeError::UnknownArg { addr, declared } => format!(
                "Refused: '{addr}' is not an argument of this tool. Its arguments are {}.",
                quote_all(declared)
            ),
            InvokeError::ArgOutOfSpace { addr, space, got } => format!(
                "Refused: argument '{addr}' must be {}; you sent '{got}'.",
                space.describe()
            ),
            InvokeError::RequiredArgsUnbound { addrs } => {
                format!("Refused: required arguments missing: {}.", quote_all(addrs))
            }
            InvokeError::UninvocableArg { addr } => format!(
                "Refused: argument '{addr}' cannot take the value sent. A tree argument takes a JSON object with a \"kind\"; an action is bound by the host and never by a caller."
            ),
            InvokeError::BodyFailed { reason } => {
                format!("Refused: the tool ran and failed: {reason}.")
            }
            InvokeError::NonTotalCapability { id, addrs } => format!(
                "Refused: the tool '{id}' cannot be registered, because it is not total: {} must each be a repeat over a bounded count, or a declared hole.",
                quote_all(addrs)
            ),
            InvokeError::IllFormedCapability { id, fault } => format!(
                "Refused: the tool '{id}' cannot be registered, because its declaration is ill-formed: {}.",
                fault.describe()
            ),
            InvokeError::DuplicateArg { addr } => format!(
                "Refused: argument '{addr}' is given more than once. Give each argument once."
            ),
            InvokeError::PolicyRefused {
                policy,
                reason,
                allowed,
            } if allowed.is_empty() => {
                format!("Refused: the policy '{policy}' does not allow this: {reason}.")
            }
            InvokeError::PolicyRefused {
                policy,
                reason,
                allowed,
            } => format!(
                "Refused: the policy '{policy}' does not allow this: {reason}. What it allows instead: {}.",
                quote_all(allowed)
            ),
            InvokeError::ApprovalRequired { policy } => format!(
                "Refused: the policy '{policy}' requires an approval before this tool runs."
            ),
        }
    }
}

/// THE admission gate both registries and the declaration reader run: totality
/// first (`NonTotalCapability`, naming every non-total entry), then
/// well-formedness (`IllFormedCapability`): the signature's, then the declared
/// result's (`EmptyResult`, `NonFiniteResult`). `None` admits. Twin of
/// `Capability.admissionOf`; with `&Returns::Nothing` it is the gate over the
/// signature alone (`Capability.signatureAdmissionOf`).
pub fn admission_of(id: &str, sg: &Signature, result: &Returns) -> Option<InvokeError> {
    let non_total: Vec<String> = sg
        .holes
        .iter()
        .filter(|e| !e.is_total())
        .map(|e| e.addr.clone())
        .collect();
    if !non_total.is_empty() {
        return Some(InvokeError::NonTotalCapability {
            id: id.to_string(),
            addrs: non_total,
        });
    }
    sg.validate()
        .err()
        .or_else(|| result.fault())
        .map(|fault| InvokeError::IllFormedCapability {
            id: id.to_string(),
            fault,
        })
}

/// Every address an argument list binds more than once, at each repeat, in list
/// order, whatever the bound values are. Twin of `Capability.repeatedAddrs`, which
/// the argument check and the pipeline type check both run.
pub fn repeated_addrs<V>(args: &[(String, V)]) -> Vec<String> {
    let mut seen: Vec<&str> = Vec::new();
    let mut dups = Vec::new();
    for (addr, _) in args {
        if seen.contains(&addr.as_str()) {
            dups.push(addr.clone());
        } else {
            seen.push(addr);
        }
    }
    dups
}

// ─── The capture key and the invocation's identity ───────────────────────────

impl Capability {
    /// Build a capability. There is no capability without a declared result:
    /// [`Returns::Nothing`] is how one that answers no value says so. Twin of
    /// `Capability.create`.
    pub fn new(
        id: impl Into<String>,
        signature: Signature,
        placement: Placement,
        result: Returns,
    ) -> Capability {
        Capability {
            id: id.into(),
            signature,
            result,
            placement,
        }
    }

    /// The capture-keying axis: the signature's effect determinism, derived. Twin
    /// of `Capability.Determinism`.
    pub fn determinism(&self) -> DeterminismSet {
        self.signature.effect.determinism
    }

    /// The determinism label this capability keys its captures on. Twin of
    /// `Capability.determinismTag`.
    pub fn determinism_tag(&self) -> String {
        self.determinism().tag()
    }

    fn hole(&self, addr: &str) -> Option<&SigEntry> {
        self.signature.holes.iter().find(|h| h.addr == addr)
    }

    /// Every hole address, in declaration order — what an `UnknownArg` names.
    fn declared(&self) -> Vec<String> {
        self.signature
            .holes
            .iter()
            .map(|h| h.addr.clone())
            .collect()
    }

    /// The fields of an invocation's canonical pre-image after the id, written
    /// through `out` as they are produced: each binding's address, then its
    /// value's canonical spelling in its hole's space where it lies there, else
    /// its own spelling, the bindings in address order (ordinal; equal addresses
    /// in list order, as the reference's stable `List.sortBy` keeps them). Twin
    /// of `Capability.bindingFields`; the key hashes these fields and the
    /// identity keeps them.
    ///
    /// Nothing is built but a document's text (DECISIONS.md D12, D13): the order is found by selection, each
    /// pass taking the least binding after the one last written, so it costs
    /// `O(n²)` comparisons of addresses and no vector; a number is spelled
    /// through `core::fmt` into the field ([`Respelled`]); a document in a
    /// document hole is written as its canonical text ([`document_of`]), which
    /// reads and renders it, the one binding here that builds anything (D13);
    /// and every other value is written as sent, which is its canonical spelling
    /// where it lies in its space and its own spelling where it does not (a tree
    /// argument is never parsed here).
    fn write_binding_fields<W: Write + ?Sized>(
        &self,
        args: &[(String, String)],
        out: &mut W,
    ) -> fmt::Result {
        // The binding at `i` comes after the one at `j` in the pre-image's order.
        let after = |i: usize, j: usize| match ordinal_cmp(&args[i].0, &args[j].0) {
            Ordering::Greater => true,
            Ordering::Equal => i > j,
            Ordering::Less => false,
        };
        let mut last: Option<usize> = None;
        while let Some(at) = (0..args.len())
            .filter(|&i| last.is_none_or(|l| after(i, l)))
            .reduce(|best, i| if after(best, i) { i } else { best })
        {
            let (addr, value) = &args[at];
            write_canonical_field(addr, out)?;
            let space = match self.hole(addr).map(|h| &h.kind) {
                Some(HoleKind::Value(sp) | HoleKind::Repeat(sp)) => Some(sp),
                _ => None,
            };
            match space {
                Some(ValueSpace::Document(_)) => match document_of(value) {
                    Some(text) => FieldBody(&mut *out).write_str(&text)?,
                    None => FieldBody(&mut *out).write_str(value)?,
                },
                Some(sp) => match sp.respelled(value) {
                    Some(r) => write!(FieldBody(&mut *out), "{r}")?,
                    None => FieldBody(&mut *out).write_str(value)?,
                },
                None => FieldBody(&mut *out).write_str(value)?,
            }
            out.write_char(FOLD_SEP)?;
            last = Some(at);
        }
        Ok(())
    }

    /// The effect-identity key a non-deterministic invocation is journalled and
    /// replayed under: the id, `#`, and the FNV-1a of the canonical pre-image of
    /// the addr-sorted (ordinal) bindings, two fields each, every value in its
    /// canonical spelling in its hole's space where it lies there. A 32-bit cache
    /// hash: never what a grant is bound to ([`Capability::invocation_identity`]).
    /// Twin of `Capability.invocationKey`.
    pub fn invocation_key(&self, args: &[(String, String)]) -> String {
        let mut preimage = String::new();
        self.write_binding_fields(args, &mut preimage)
            .expect("a String accepts every write");
        keyed_by_preimage(&self.id, &preimage)
    }

    /// The IDENTITY of one invocation, what a [`Grant`] is for: the capability id,
    /// then the fields the key hashes, as one canonical pre-image
    /// ([`canonical_fields`](crate::hash::canonical_fields): every U+0010 and
    /// U+0001 a field holds preceded by U+0010, each field terminated by U+0001),
    /// with NO hash taken. Injective: two invocations share an identity only when
    /// they name one capability and carry the same canonical bindings. Compared
    /// as bytes, never parsed. Twin of `Capability.invocationPreimage`.
    ///
    /// The one allocation is the answer, sized exactly: the identity is written
    /// once to measure it and once into the string. Deciding builds nothing:
    /// [`Grant::covers_invocation`] compares a grant with the identity as it is
    /// written, and [`Capability::write_invocation_identity`] writes it into a
    /// host's own buffer (DECISIONS.md D12).
    pub fn invocation_identity(&self, args: &[(String, String)]) -> String {
        let mut measured = Measured(0);
        self.write_invocation_identity(args, &mut measured)
            .expect("measuring accepts every write");
        let mut identity = String::with_capacity(measured.0);
        self.write_invocation_identity(args, &mut identity)
            .expect("a String accepts every write");
        identity
    }

    /// [`Capability::invocation_identity`] written through `out` as it is
    /// produced, building nothing: the form a host with no allocation to spare
    /// writes the identity in, into a buffer of its own, to mint or to log a
    /// grant. `out`'s refusal stops the write and is answered.
    pub fn write_invocation_identity<W: Write + ?Sized>(
        &self,
        args: &[(String, String)],
        out: &mut W,
    ) -> fmt::Result {
        write_canonical_field(&self.id, out)?;
        self.write_binding_fields(args, out)
    }

    /// The grant for exactly this invocation: what a host mints when an approval
    /// is given for a previewed invocation. Twin of `Capability.grant`.
    pub fn grant(&self, args: &[(String, String)]) -> Grant {
        Grant::of_invocation(self.invocation_identity(args))
    }

    /// The refusal one argument earns on its own, decided on the value: the
    /// space a refusal names is copied only when the argument is refused, and a
    /// tree argument's document is read once. Twin of `Capability.argFault`.
    fn arg_fault(&self, addr: &str, value: &str) -> Option<InvokeError> {
        let Some(h) = self.hole(addr) else {
            return Some(InvokeError::UnknownArg {
                addr: addr.to_string(),
                declared: self.declared(),
            });
        };
        let uninvocable = || InvokeError::UninvocableArg {
            addr: addr.to_string(),
        };
        let out_of_space = |space: ValueSpace| InvokeError::ArgOutOfSpace {
            addr: addr.to_string(),
            space,
            got: value.to_string(),
        };
        // A tree argument: not a tree at all is uninvocable, a tree of another
        // kind is out of the space.
        let tree = |c: &Option<String>| match slot_kind_of(value) {
            None => Some(uninvocable()),
            Some(k) if c.as_ref().is_none_or(|c| *c == k) => None,
            Some(_) => Some(out_of_space(ValueSpace::SlotTree(c.clone()))),
        };
        match &h.kind {
            HoleKind::Action(_) => Some(uninvocable()),
            HoleKind::Slot(c)
            | HoleKind::Value(ValueSpace::SlotTree(c))
            | HoleKind::Repeat(ValueSpace::SlotTree(c)) => tree(c),
            HoleKind::Value(sp) | HoleKind::Repeat(sp) => {
                (!sp.validate(value)).then(|| out_of_space(sp.clone()))
            }
        }
    }

    /// The required holes `args` leaves unbound, in declaration order; building
    /// nothing when every one is bound.
    fn unbound<V>(&self, args: &[(String, V)]) -> Vec<String> {
        self.signature
            .holes
            .iter()
            .filter(|h| h.required && !args.iter().any(|(a, _)| *a == h.addr))
            .map(|h| h.addr.clone())
            .collect()
    }

    /// Validate `args` (addr → string value) before dispatch, default-deny by
    /// shape: every address bound once (`DuplicateArg`), every argument at a
    /// declared hole (`UnknownArg`) that takes a caller's value
    /// (`UninvocableArg`) in its space (`ArgOutOfSpace`), then every required hole
    /// bound (`RequiredArgsUnbound`). The first refusal wins. Membership is decided
    /// on the value and a refusal is built only when one is answered, so an
    /// admitted invocation builds nothing unless it carries a tree argument
    /// (DECISIONS.md D12). Twin of `Capability.validateArgs`.
    pub fn validate_args(&self, args: &[(String, String)]) -> Result<(), InvokeError> {
        // The first binding, in list order, whose address an earlier one binds:
        // the head of `repeated_addrs`, found without collecting it.
        let repeat = args
            .iter()
            .enumerate()
            .find(|(i, (a, _))| args[..*i].iter().any(|(b, _)| b == a));
        if let Some((_, (addr, _))) = repeat {
            return Err(InvokeError::DuplicateArg { addr: addr.clone() });
        }
        if let Some(e) = args.iter().find_map(|(a, v)| self.arg_fault(a, v)) {
            return Err(e);
        }
        let unbound = self.unbound(args);
        if unbound.is_empty() {
            Ok(())
        } else {
            Err(InvokeError::RequiredArgsUnbound { addrs: unbound })
        }
    }

    /// Validate as [`Capability::validate_args`] does, answering EVERY refusal:
    /// each repeated address, then each refused argument in order, then the
    /// unbound required holes. Its head is `validate_args`'s refusal. Twin of
    /// `Capability.validateArgsAll`.
    pub fn validate_args_all(&self, args: &[(String, String)]) -> Result<(), Vec<InvokeError>> {
        let mut all: Vec<InvokeError> = repeated_addrs(args)
            .into_iter()
            .map(|addr| InvokeError::DuplicateArg { addr })
            .collect();
        all.extend(args.iter().filter_map(|(a, v)| self.arg_fault(a, v)));
        let unbound = self.unbound(args);
        if !unbound.is_empty() {
            all.push(InvokeError::RequiredArgsUnbound { addrs: unbound });
        }
        if all.is_empty() { Ok(()) } else { Err(all) }
    }
}

// ─── The declaration codec ───────────────────────────────────────────────────

fn obj(fields: Vec<(&str, JVal)>) -> JVal {
    JVal::Obj(
        fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
    )
}

fn typed(tag: &str, mut fields: Vec<(&str, JVal)>) -> JVal {
    fields.insert(0, ("$type", JVal::Str(tag.to_string())));
    obj(fields)
}

fn jstr(s: &str) -> JVal {
    JVal::Str(s.to_string())
}

/// A value space as its wire document (`"$type"`, `min` / `max`, `values`,
/// `slotKind`, `documentType`). Twin of `SpaceCodec.toJson`.
pub fn space_to_json(s: &ValueSpace) -> JVal {
    match s {
        ValueSpace::IntRange(lo, hi) => typed(
            "intRange",
            vec![
                ("min", JVal::Num((*lo).into())),
                ("max", JVal::Num((*hi).into())),
            ],
        ),
        ValueSpace::FloatRange(lo, hi) => typed(
            "floatRange",
            vec![("min", JVal::Num(*lo)), ("max", JVal::Num(*hi))],
        ),
        ValueSpace::StringLen(lo, hi) => typed(
            "stringLen",
            vec![
                ("min", JVal::Num((*lo).into())),
                ("max", JVal::Num((*hi).into())),
            ],
        ),
        ValueSpace::Enum(xs) => typed(
            "enum",
            vec![("values", JVal::Arr(xs.iter().map(|x| jstr(x)).collect()))],
        ),
        ValueSpace::AnyString => typed("anyString", vec![]),
        ValueSpace::SlotTree(c) => typed(
            "slotTree",
            c.iter().map(|k| ("slotKind", jstr(k))).collect(),
        ),
        ValueSpace::Document(c) => typed(
            "document",
            c.iter().map(|t| ("documentType", jstr(t))).collect(),
        ),
    }
}

/// An effect class as its untagged two-member object. Twin of `EffectCodec.toJson`.
pub fn effect_to_json(e: &EffectClass) -> JVal {
    obj(vec![
        ("host", jstr(e.host.tag())),
        ("determinism", JVal::Str(e.determinism.tag())),
    ])
}

fn entry_to_json(e: &SigEntry) -> JVal {
    let mut fields = vec![
        ("addr", jstr(&e.addr)),
        ("name", jstr(&e.name)),
        ("kind", jstr(e.kind.tag())),
        ("required", JVal::Bool(e.required)),
    ];
    match &e.kind {
        HoleKind::Value(s) | HoleKind::Repeat(s) => fields.push(("space", space_to_json(s))),
        HoleKind::Slot(Some(c)) => fields.push(("slotKind", jstr(c))),
        HoleKind::Slot(None) => {}
        HoleKind::Action(eff) => fields.push(("actionEffect", effect_to_json(eff))),
    }
    obj(fields)
}

fn signature_to_json(sg: &Signature) -> JVal {
    obj(vec![
        ("name", jstr(&sg.name)),
        ("effect", effect_to_json(&sg.effect)),
        (
            "holes",
            JVal::Arr(sg.holes.iter().map(entry_to_json).collect()),
        ),
    ])
}

/// A declared result as its wire document: a value's space as the space document
/// itself, and "returns nothing" as `{"$type":"nothing"}`, a tag no value space
/// uses. Twin of `CapabilityCodec.resultJson`.
pub fn result_to_json(r: &Returns) -> JVal {
    match r {
        Returns::Nothing => typed("nothing", vec![]),
        Returns::Value(space) => space_to_json(space),
    }
}

/// A placement as its `$type`-tagged document. Twin of `CapabilitySeamCodec.encodePlacement`.
pub fn placement_to_json(p: Placement) -> JVal {
    match p {
        Placement::ClientIsland(k) => typed("clientIsland", vec![("island", jstr(k.tag()))]),
        other => typed(other.tag(), vec![]),
    }
}

impl Capability {
    /// The declaration as a `"$type":"capability"` value. Twin of `CapabilityCodec.encodeJson`.
    pub fn encode_json(&self) -> JVal {
        typed(
            "capability",
            vec![
                ("id", jstr(&self.id)),
                ("signature", signature_to_json(&self.signature)),
                ("result", result_to_json(&self.result)),
                ("determinism", JVal::Str(self.determinism_tag())),
                ("placement", placement_to_json(self.placement)),
            ],
        )
    }

    /// The declaration's canonical text: one capability, one string. Total; a
    /// non-finite bound is written as its quoted sentinel, which no reader takes
    /// back — use [`Capability::try_encode`] for a declaration nothing has
    /// validated. Twin of `CapabilityCodec.encode`.
    pub fn encode(&self) -> String {
        render_canonical(&self.encode_json())
    }

    /// The guarded encode: refuses a declaration with a non-finite bound, in a
    /// hole's space or in the declared result's, rather than write bytes the
    /// reader refuses; `Ok` is exactly [`Capability::encode`]. Twin of
    /// `CapabilityCodec.tryEncode`.
    pub fn try_encode(&self) -> Result<String, String> {
        let result = match &self.result {
            Returns::Value(space) => Some(space.clone()),
            Returns::Nothing => None,
        };
        let spaces = self.signature.holes.iter().map(SigEntry::space);
        for space in spaces.chain([result]) {
            if let Some(ValueSpace::FloatRange(lo, hi)) = space {
                for b in [lo, hi] {
                    if !b.is_finite() {
                        return Err(format!(
                            "a non-finite number ({}) has no canonical JSON spelling",
                            crate::number::format_number(b)
                        ));
                    }
                }
            }
        }
        Ok(self.encode())
    }

    /// Read a declaration from its text (strict JSON, then [`Capability::decode_value`]).
    /// Twin of `CapabilityCodec.decodeDetailedWith ReadPolicy.Lenient`.
    pub fn decode(text: &str) -> Result<Capability, DecodeError> {
        let v = decode::parse(text, &JsonLimits::STRICT)?;
        Capability::decode_value(&Cursor::root(&v))
    }

    /// Read a declaration leniently (an unknown member is read past): its
    /// `"$type"` must be `capability`; its signature must pass the admission gate
    /// ([`admission_of`], refused `OutOfRange` at `signature.holes` with the
    /// registry's sentence); its `result` is REQUIRED (absent is `MissingField` at
    /// `result`) and held to the same gate (`OutOfRange` at `result`), after the
    /// signature, so a signature fault is answered first; and its wire
    /// `determinism` must equal the label its signature's effect derives
    /// (`OutOfRange` at `determinism`). Twin of `CapabilityCodec.decodeJson`.
    pub fn decode_value(c: &Cursor<'_>) -> Result<Capability, DecodeError> {
        let refused = |at: &Cursor<'_>, e: InvokeError| {
            at.fail(
                DecodeCode::OutOfRange,
                format!("a declaration the registries admit: {}", e.describe()),
            )
        };
        c.tag("$type", &["capability"])?;
        let id = c.field("id")?.string()?.to_string();
        let sig_at = c.field("signature")?;
        let signature = signature_of(&sig_at)?;
        if let Some(e) = admission_of(&id, &signature, &Returns::Nothing) {
            return Err(refused(&sig_at.field("holes")?, e));
        }
        let result_at = c.field("result")?;
        let result = result_of(&result_at)?;
        if let Some(e) = admission_of(&id, &signature, &result) {
            return Err(refused(&result_at, e));
        }
        let expected = signature.effect.determinism.tag();
        let det_at = c.field("determinism")?;
        let wire = det_at.string()?;
        if wire != expected {
            return Err(det_at.fail(
                DecodeCode::OutOfRange,
                format!(
                    "capability determinism disagrees with signature effect: wire '{wire}' vs signature '{expected}'"
                ),
            ));
        }
        let placement = placement_of(&c.field("placement")?)?;
        Ok(Capability {
            id,
            signature,
            result,
            placement,
        })
    }
}

fn one_of<'a, T: Copy>(at: &Cursor<'a>, table: &[(&str, T)], what: &str) -> Result<T, DecodeError> {
    let s = at.string()?;
    table
        .iter()
        .find(|(t, _)| *t == s)
        .map(|(_, v)| *v)
        .ok_or_else(|| {
            let names: Vec<&str> = table.iter().map(|(t, _)| *t).collect();
            at.fail(
                DecodeCode::UnknownTag,
                format!("unknown {what}: {s}; expected one of {}", quote_all(&names)),
            )
        })
}

/// The value spaces' `"$type"` tags, in the reference's case order.
const SPACE_TAGS: [&str; 7] = [
    "intRange",
    "floatRange",
    "stringLen",
    "enum",
    "anyString",
    "slotTree",
    "document",
];

/// Read a value space from its wire document, dispatched on `"$type"`. Twin of
/// `SpaceCodec.decoder`.
pub fn space_of(c: &Cursor<'_>) -> Result<ValueSpace, DecodeError> {
    let tag = c.tag("$type", &SPACE_TAGS)?;
    Ok(match tag {
        "intRange" => ValueSpace::IntRange(c.field("min")?.int()?, c.field("max")?.int()?),
        "floatRange" => ValueSpace::FloatRange(c.field("min")?.float()?, c.field("max")?.float()?),
        "stringLen" => ValueSpace::StringLen(c.field("min")?.int()?, c.field("max")?.int()?),
        "enum" => ValueSpace::Enum(
            c.field("values")?
                .items()?
                .iter()
                .map(|i| i.string().map(String::from))
                .collect::<Result<_, _>>()?,
        ),
        "anyString" => ValueSpace::AnyString,
        "slotTree" => ValueSpace::SlotTree(optional_str(c, "slotKind")?),
        _ => ValueSpace::Document(optional_str(c, "documentType")?),
    })
}

/// An optional string member: absent is `None`, present must be a string.
fn optional_str(c: &Cursor<'_>, name: &str) -> Result<Option<String>, DecodeError> {
    c.optional(name)?
        .map(|v| v.string().map(String::from))
        .transpose()
}

/// Read a declared result as [`result_to_json`] writes it, dispatched on
/// `"$type"`: `nothing`, or a value space's tag read by [`space_of`]. Any other
/// tag is `UnknownTag`; JSON `null` is no spelling of "nothing". Twin of
/// `CapabilityCodec.resultOf`.
pub fn result_of(c: &Cursor<'_>) -> Result<Returns, DecodeError> {
    let mut known = SPACE_TAGS.to_vec();
    known.push("nothing");
    match c.tag("$type", &known)? {
        "nothing" => Ok(Returns::Nothing),
        _ => space_of(c).map(Returns::Value),
    }
}

/// Read an effect class. Twin of `EffectCodec.decoder`.
pub fn effect_of(c: &Cursor<'_>) -> Result<EffectClass, DecodeError> {
    let hosts: Vec<(&str, HostEffect)> = HostEffect::ALL.iter().map(|h| (h.tag(), *h)).collect();
    let host = one_of(&c.field("host")?, &hosts, "host effect")?;
    let det_at = c.field("determinism")?;
    let tag = det_at.string()?;
    let determinism = DeterminismSet::of_tag(tag).ok_or_else(|| {
        det_at.fail(
            DecodeCode::UnknownTag,
            format!("unknown determinism: {tag}"),
        )
    })?;
    Ok(EffectClass { host, determinism })
}

fn entry_of(c: &Cursor<'_>) -> Result<SigEntry, DecodeError> {
    let addr = c.field("addr")?.string()?.to_string();
    let name = c.field("name")?.string()?.to_string();
    let kind_at = c.field("kind")?;
    let kind = one_of(
        &kind_at,
        &[("value", 0u8), ("slot", 1), ("repeat", 2), ("action", 3)],
        "hole kind",
    )?;
    let required = c.field("required")?.boolean()?;
    let space = c.optional("space")?.map(|s| space_of(&s)).transpose()?;
    let action = c
        .optional("actionEffect")?
        .map(|e| effect_of(&e))
        .transpose()?;
    let slot = optional_str(c, "slotKind")?;
    let kind = match (kind, space, action) {
        (0, Some(s), _) => HoleKind::Value(s),
        (2, Some(s), _) => HoleKind::Repeat(s),
        (1, _, _) => HoleKind::Slot(slot),
        (3, _, Some(e)) => HoleKind::Action(e),
        (3, _, None) => return Err(c.field("actionEffect").unwrap_err()),
        _ => return Err(c.field("space").unwrap_err()),
    };
    Ok(SigEntry {
        addr,
        name,
        kind,
        required,
    })
}

/// A signature's members, read with no admission check (the declaration reader
/// runs the gate over it). Twin of `CapabilityCodec.signatureFieldsOf`.
fn signature_of(c: &Cursor<'_>) -> Result<Signature, DecodeError> {
    let name = c.field("name")?.string()?.to_string();
    let effect = effect_of(&c.field("effect")?)?;
    let holes = c
        .field("holes")?
        .items()?
        .iter()
        .map(entry_of)
        .collect::<Result<_, _>>()?;
    Ok(Signature {
        name,
        holes,
        effect,
    })
}

/// Read a placement. Twin of `CapabilitySeamCodec.decodePlacement`.
pub fn placement_of(c: &Cursor<'_>) -> Result<Placement, DecodeError> {
    let tag = c.tag(
        "$type",
        &[
            "buildTime",
            "server",
            "clientDeclarative",
            "precomputed",
            "clientIsland",
        ],
    )?;
    Ok(match tag {
        "buildTime" => Placement::BuildTime,
        "server" => Placement::Server,
        "clientDeclarative" => Placement::ClientDeclarative,
        "precomputed" => Placement::Precomputed,
        _ => {
            let islands: Vec<(&str, IslandKind)> =
                IslandKind::ALL.iter().map(|k| (k.tag(), *k)).collect();
            Placement::ClientIsland(one_of(&c.field("island")?, &islands, "island kind")?)
        }
    })
}

/// A typed invocation `(capabilityId, args)` as its canonical
/// `"$type":"invocation"` text, the arguments in the order given. Twin of
/// `CapabilityCodec.encodeInvocation`.
pub fn encode_invocation(capability_id: &str, args: &[(String, String)]) -> String {
    let items = args
        .iter()
        .map(|(a, v)| obj(vec![("addr", jstr(a)), ("value", jstr(v))]))
        .collect();
    render_canonical(&typed(
        "invocation",
        vec![
            ("capabilityId", jstr(capability_id)),
            ("args", JVal::Arr(items)),
        ],
    ))
}

// ─── The gate: its decision, its decision context and the grant ─────────────

/// What a gate decides for one invocation under one context. Ordered by how much
/// it refuses, `Allow < NeedsApproval < Deny`; a registry's decision is the join
/// (the most refusing) over its gates, and a denial is GUIDANCE: the message and
/// the alternatives a refused caller may take instead. Twin of `PolicyDecision`.
///
/// The guidance is SHARED text (`Arc<str>`), the twin of the reference's
/// immutable strings: a gate builds it once, when the gate is built, and every
/// denial it makes carries it by a reference count, so deciding copies nothing
/// and allocates nothing (DECISIONS.md D12). A gate that builds its guidance
/// inside its decision allocates there, on every denial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    /// The invocation may proceed now.
    Allow,
    /// The invocation must be approved first: a grant for exactly it lifts this
    /// to `Allow`.
    NeedsApproval,
    /// The invocation is refused. No grant lifts it.
    Deny {
        message: Arc<str>,
        alternatives: Arc<[Arc<str>]>,
    },
}

impl PolicyDecision {
    /// A denial with `reason` and no alternatives. Twin of `PolicyDecision.deny`.
    pub fn deny(reason: impl Into<Arc<str>>) -> PolicyDecision {
        PolicyDecision::deny_with(reason, Arc::<[Arc<str>]>::default())
    }

    /// A denial with `reason` and the alternatives the caller may take instead.
    /// Twin of `PolicyDecision.denyWith`.
    pub fn deny_with(
        reason: impl Into<Arc<str>>,
        alternatives: impl Into<Arc<[Arc<str>]>>,
    ) -> PolicyDecision {
        PolicyDecision::Deny {
            message: reason.into(),
            alternatives: alternatives.into(),
        }
    }

    /// The decision's place in `Allow < NeedsApproval < Deny`. Twin of
    /// `PolicyDecision.rank`.
    pub fn rank(&self) -> u8 {
        match self {
            PolicyDecision::Allow => 0,
            PolicyDecision::NeedsApproval => 1,
            PolicyDecision::Deny { .. } => 2,
        }
    }

    /// The decision once an approval is read: `NeedsApproval` becomes `Allow`
    /// when `granted`, and every other decision is returned as it is, so no grant
    /// lifts a `Deny` or changes an `Allow`. Twin of `PolicyDecision.lifted`.
    pub fn lifted(self, granted: bool) -> PolicyDecision {
        match self {
            PolicyDecision::NeedsApproval if granted => PolicyDecision::Allow,
            d => d,
        }
    }
}

/// An approval presented at a dispatch, bound to ONE invocation by that
/// invocation's identity ([`Capability::invocation_identity`]), never by its
/// hashed capture key. The core checks one thing: that the identity it carries
/// is this invocation's, byte for byte. Who minted it, whether it was spent,
/// when it expires are the host's, as facts it puts in the decision context,
/// where a gate reads them and can answer `Deny`. Twin of `Grant`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Grant {
    invocation: String,
}

impl Grant {
    /// The grant for the invocation `invocation` names. Twin of `Grant.ofInvocation`.
    pub fn of_invocation(invocation: impl Into<String>) -> Grant {
        Grant {
            invocation: invocation.into(),
        }
    }

    /// The identity of the invocation the grant is for.
    pub fn invocation(&self) -> &str {
        &self.invocation
    }

    /// Is the grant for exactly the invocation `identity` names: the two
    /// identities equal as bytes, never normalised or parsed. Twin of `Grant.covers`.
    pub fn covers(&self, identity: &str) -> bool {
        self.invocation.as_bytes() == identity.as_bytes()
    }

    /// Is the grant for exactly the invocation of `c` with `args`: what
    /// `covers(&c.invocation_identity(args))` answers, with the identity compared
    /// with the grant's bytes AS IT IS WRITTEN, stopping at the first byte that
    /// differs, and never built (DECISIONS.md D12). What a dispatch asks.
    pub fn covers_invocation(&self, c: &Capability, args: &[(String, String)]) -> bool {
        let mut rest = Unmatched(self.invocation.as_bytes());
        c.write_invocation_identity(args, &mut rest).is_ok() && rest.0.is_empty()
    }
}

/// The bytes of a grant not yet matched by the identity being written: each
/// write must be their prefix, and a write that is not refuses, which stops the
/// identity there.
struct Unmatched<'a>(&'a [u8]);

impl Write for Unmatched<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.0 = self.0.strip_prefix(s.as_bytes()).ok_or(fmt::Error)?;
        Ok(())
    }
}

/// The length in bytes of what is written through it; it keeps nothing.
struct Measured(usize);

impl Write for Measured {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.0 += s.len();
        Ok(())
    }
}

/// A gate's decision function: the decision context, the resolved declaration
/// and the validated arguments. Total and free of side effects (a registry is
/// asked to decide without dispatching), so it is shareable across threads.
type Decide<C> = dyn Fn(&C, &Capability, &[(String, String)]) -> PolicyDecision + Send + Sync;

/// One named gate over a registry's invocations: its name, carried by every
/// refusal it makes, and its decision under a context `C`. The context is the
/// host's type; the core names nothing it holds. The name is shared text, so a
/// refusal carries it by a reference count and never a copy (DECISIONS.md D12).
/// Twin of `PolicyGate`.
pub struct PolicyGate<C> {
    policy: Arc<str>,
    decide: Arc<Decide<C>>,
}

impl<C> PolicyGate<C> {
    /// A gate named `policy` deciding with `decide`.
    pub fn new(
        policy: impl Into<Arc<str>>,
        decide: impl Fn(&C, &Capability, &[(String, String)]) -> PolicyDecision + Send + Sync + 'static,
    ) -> PolicyGate<C> {
        PolicyGate {
            policy: policy.into(),
            decide: Arc::new(decide),
        }
    }

    /// A gate that decides on the declaration and the arguments alone, the same
    /// under every context. Twin of `PolicyGate.ignoringContext`.
    pub fn ignoring_context(
        policy: impl Into<Arc<str>>,
        decide: impl Fn(&Capability, &[(String, String)]) -> PolicyDecision + Send + Sync + 'static,
    ) -> PolicyGate<C> {
        PolicyGate::new(
            policy,
            move |_: &C, c: &Capability, a: &[(String, String)]| decide(c, a),
        )
    }

    /// The gate's name.
    pub fn policy(&self) -> &str {
        &self.policy
    }

    /// The gate's decision for one invocation under `ctx`.
    pub fn decide(&self, ctx: &C, c: &Capability, args: &[(String, String)]) -> PolicyDecision {
        (self.decide)(ctx, c, args)
    }
}

impl<C> Clone for PolicyGate<C> {
    fn clone(&self) -> Self {
        PolicyGate {
            policy: Arc::clone(&self.policy),
            decide: Arc::clone(&self.decide),
        }
    }
}

impl<C> core::fmt::Debug for PolicyGate<C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PolicyGate")
            .field("policy", &self.policy)
            .finish_non_exhaustive()
    }
}

/// Equal when both have the same name and the SAME decision function: a
/// function has no extensional equality, so it is identity of the function.
impl<C> PartialEq for PolicyGate<C> {
    fn eq(&self, other: &Self) -> bool {
        self.policy == other.policy && Arc::ptr_eq(&self.decide, &other.decide)
    }
}

// ─── The registry ────────────────────────────────────────────────────────────

/// A typed capability registry: the discovery surface an agent enumerates,
/// default-deny on dispatch, and the gates its invocations are decided by under
/// a decision context `C` (`()` for a registry that needs none). Opaque:
/// [`CapabilityRegistry::register`] and its admission gate are the only way a
/// capability gets in, and [`CapabilityRegistry::with_gate`] the only way a gate
/// does. Twin of `CapabilityRegistry<'Ctx>` (its denial observers are not
/// carried: a refusal is the dispatch's `Err`).
pub struct CapabilityRegistry<C = ()> {
    /// Held in ordinal id order, the order `enumerate` answers in.
    entries: Vec<Capability>,
    /// In the order they run.
    gates: Vec<PolicyGate<C>>,
}

impl<C> Clone for CapabilityRegistry<C> {
    fn clone(&self) -> Self {
        CapabilityRegistry {
            entries: self.entries.clone(),
            gates: self.gates.clone(),
        }
    }
}

impl<C> Default for CapabilityRegistry<C> {
    fn default() -> Self {
        CapabilityRegistry {
            entries: Vec::new(),
            gates: Vec::new(),
        }
    }
}

impl<C> core::fmt::Debug for CapabilityRegistry<C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CapabilityRegistry")
            .field("entries", &self.entries)
            .field("gates", &self.gates)
            .finish()
    }
}

/// Equal when both hold the same capabilities and the same gates in the same
/// order (each by name and by identity of its decision function).
impl<C> PartialEq for CapabilityRegistry<C> {
    fn eq(&self, other: &Self) -> bool {
        self.entries == other.entries && self.gates == other.gates
    }
}

impl CapabilityRegistry<()> {
    /// The registry with no capabilities and no gates, deciding under no context.
    /// Twin of `CapabilityRegistry.empty` at `unit`.
    pub fn new() -> CapabilityRegistry<()> {
        CapabilityRegistry::empty()
    }
}

impl<C> CapabilityRegistry<C> {
    /// The registry with no capabilities and no gates, deciding under contexts of
    /// type `C`. Twin of `CapabilityRegistry.empty`.
    pub fn empty() -> CapabilityRegistry<C> {
        CapabilityRegistry::default()
    }

    fn position(&self, id: &str) -> Result<usize, usize> {
        self.entries.binary_search_by(|c| ordinal_cmp(&c.id, id))
    }

    /// Register a capability: a held id is `DuplicateCapability` (checked first),
    /// and the admission gate's refusal otherwise — the signature's, then the
    /// declared result's; the registry is unchanged on a refusal. Twin of
    /// `CapabilityRegistry.register`.
    pub fn register(&self, c: Capability) -> Result<CapabilityRegistry<C>, InvokeError> {
        match self.position(&c.id) {
            Ok(_) => Err(InvokeError::DuplicateCapability { id: c.id }),
            Err(at) => {
                if let Some(e) = admission_of(&c.id, &c.signature, &c.result) {
                    return Err(e);
                }
                let mut next = self.clone();
                next.entries.insert(at, c);
                Ok(next)
            }
        }
    }

    /// The capability registered under exactly `id` (ordinal). Twin of `CapabilityRegistry.tryFind`.
    pub fn try_find(&self, id: &str) -> Option<&Capability> {
        self.position(id).ok().map(|i| &self.entries[i])
    }

    /// Every capability, in ordinal id order whatever the insertion order. Twin
    /// of `CapabilityRegistry.enumerate`.
    pub fn enumerate(&self) -> &[Capability] {
        &self.entries
    }

    /// The held ids, in id order — what a `NoSuchCapability` names.
    pub fn ids(&self) -> Vec<String> {
        self.entries.iter().map(|c| c.id.clone()).collect()
    }

    /// Remove the capability under `id`; an id not held is `NoSuchCapability`
    /// naming every held id. Twin of `CapabilityRegistry.unregister`.
    pub fn unregister(&self, id: &str) -> Result<CapabilityRegistry<C>, InvokeError> {
        match self.position(id) {
            Ok(i) => {
                let mut next = self.clone();
                next.entries.remove(i);
                Ok(next)
            }
            Err(_) => Err(InvokeError::NoSuchCapability {
                id: id.to_string(),
                known: self.ids(),
            }),
        }
    }

    /// The registry with `gate` added after its gates. Adding is the only verb:
    /// the decision is the join over every gate, so the result refuses everything
    /// this one refused, and possibly more. Twin of `CapabilityRegistry.withGate`.
    pub fn with_gate(&self, gate: PolicyGate<C>) -> CapabilityRegistry<C> {
        let mut next = self.clone();
        next.gates.push(gate);
        next
    }

    /// The gate names, in the order they run. Twin of `CapabilityRegistry.gates`.
    pub fn gates(&self) -> Vec<&str> {
        self.gates.iter().map(PolicyGate::policy).collect()
    }

    /// Resolve `id` and validate `args` — the default-deny half of dispatch that
    /// runs before any gate and any body: an id not held is `NoSuchCapability`,
    /// and the arguments are held to [`Capability::validate_args`].
    pub fn resolve(&self, id: &str, args: &[(String, String)]) -> Result<&Capability, InvokeError> {
        let c = self
            .try_find(id)
            .ok_or_else(|| InvokeError::NoSuchCapability {
                id: id.to_string(),
                known: self.ids(),
            })?;
        c.validate_args(args)?;
        Ok(c)
    }

    /// The ONE judgement the named read and every dispatch share: resolve,
    /// validate, then the gates' join under `ctx` with the deciding gate (the
    /// FIRST gate of the most refusing rank, none when every gate allows), a
    /// `NeedsApproval` lifted to `Allow` when `grant` is for exactly this
    /// invocation, the gate kept. Builds nothing on an admitted invocation: the
    /// gate is borrowed and the grant is compared with the identity as it is
    /// written (DECISIONS.md D12). Twin of `KeyedRegistry.judge` over
    /// `RegistryPolicy.decideGranted`.
    fn judge(
        &self,
        ctx: &C,
        grant: Option<&Grant>,
        id: &str,
        args: &[(String, String)],
    ) -> Result<(&Capability, Option<&PolicyGate<C>>, PolicyDecision), InvokeError> {
        let c = self.resolve(id, args)?;
        let mut decider = None;
        let mut decision = PolicyDecision::Allow;
        for g in &self.gates {
            let d = g.decide(ctx, c, args);
            if d.rank() > decision.rank() {
                decider = Some(g);
                decision = d;
            }
        }
        // The identity is compared only when a gate asks for an approval.
        if decision == PolicyDecision::NeedsApproval {
            let granted = grant.is_some_and(|g| g.covers_invocation(c, args));
            decision = decision.lifted(granted);
        }
        Ok((c, decider, decision))
    }

    /// The registry's NAMED decision for an invocation, without dispatching it:
    /// `Ok` of the deciding gate's name, borrowed from the registry (`""` when
    /// every gate allows), and the decision (a `NeedsApproval` the grant is for
    /// lifted to `Allow`, the name kept, so it still says whose approval was
    /// answered; no grant lifts a `Deny`), or, where no gate decided, the refusal
    /// a dispatch gives: `NoSuchCapability` naming the held ids, or the
    /// validation refusal. Advisory: [`CapabilityRegistry::dispatch`] judges
    /// again, through the same function, and is authoritative. Twin of
    /// `CapabilityRegistry.decideNamed`.
    pub fn decide_named(
        &self,
        ctx: &C,
        grant: Option<&Grant>,
        id: &str,
        args: &[(String, String)],
    ) -> Result<(&str, PolicyDecision), InvokeError> {
        self.judge(ctx, grant, id, args)
            .map(|(_, gate, d)| (gate.map_or("", PolicyGate::policy), d))
    }

    /// Dispatch an invocation: resolve the id (default-deny — an id the registry
    /// does not hold is `NoSuchCapability`, whatever is presented), validate the
    /// arguments, decide under `ctx` (a `Deny` is `PolicyRefused`, whatever is
    /// presented; a `NeedsApproval` is `ApprovalRequired` unless `grant` is for
    /// exactly this invocation), and only then hand the resolved capability to
    /// `perform`, the host's body, exactly once; its refusal is the dispatch's. A
    /// refused dispatch never runs `perform`. A refusal shares the deciding gate's
    /// name and its guidance with the gate. Twin of `CapabilityRegistry.dispatch`.
    pub fn dispatch<T>(
        &self,
        ctx: &C,
        grant: Option<&Grant>,
        id: &str,
        args: &[(String, String)],
        perform: impl FnOnce(&Capability) -> Result<T, InvokeError>,
    ) -> Result<T, InvokeError> {
        let (c, gate, decision) = self.judge(ctx, grant, id, args)?;
        // Only a deciding gate refuses, so a refusal always has one to name.
        let policy = || gate.map_or_else(|| Arc::from(""), |g| Arc::clone(&g.policy));
        match decision {
            PolicyDecision::Allow => perform(c),
            PolicyDecision::NeedsApproval => {
                Err(InvokeError::ApprovalRequired { policy: policy() })
            }
            PolicyDecision::Deny {
                message,
                alternatives,
            } => Err(InvokeError::PolicyRefused {
                policy: policy(),
                reason: message,
                allowed: alternatives,
            }),
        }
    }

    /// The registry as the one context `ctx` sees it: every gate decides as it
    /// does under `ctx`, and the result takes no context. It fixes a context and
    /// loosens nothing: the view a host hands a caller that must not choose its
    /// own context. Twin of `CapabilityRegistry.under`.
    pub fn under(&self, ctx: C) -> CapabilityRegistry<()>
    where
        C: Send + Sync + 'static,
    {
        let ctx = Arc::new(ctx);
        let gates = self
            .gates
            .iter()
            .map(|g| {
                let (ctx, decide) = (Arc::clone(&ctx), Arc::clone(&g.decide));
                PolicyGate::new(
                    Arc::clone(&g.policy),
                    move |_: &(), c: &Capability, a: &[(String, String)]| decide(&ctx, c, a),
                )
            })
            .collect();
        CapabilityRegistry {
            entries: self.entries.clone(),
            gates,
        }
    }
}

// ─── The pipeline type check ─────────────────────────────────────────────────

/// Where an invoke node's argument comes from. Twin of `ArgSource`.
#[derive(Debug, Clone, PartialEq)]
pub enum ArgSource {
    /// A string value, as declared; wire tag `literal`.
    Literal(String),
    /// The output of the named node; wire tag `fromNode`.
    FromNode(String),
}

/// One node of a pipeline. Twin of `PipelineNode`.
#[derive(Debug, Clone, PartialEq)]
pub enum PipelineNode {
    /// A value the host supplies, under `data_ref`; wire tag `source`.
    Source {
        id: String,
        data_ref: String,
        output_type: ValueSpace,
    },
    /// A call of a capability; its declared output is checked against the
    /// capability's declared result; wire tag `invoke`.
    Invoke {
        id: String,
        capability_id: String,
        output_type: ValueSpace,
        args: Vec<(String, ArgSource)>,
    },
}

impl PipelineNode {
    /// The node's id. Twin of `CapabilityPipeline.nodeId`.
    pub fn id(&self) -> &str {
        match self {
            PipelineNode::Source { id, .. } | PipelineNode::Invoke { id, .. } => id,
        }
    }

    /// The node's declared output space.
    pub fn output_type(&self) -> &ValueSpace {
        match self {
            PipelineNode::Source { output_type, .. } | PipelineNode::Invoke { output_type, .. } => {
                output_type
            }
        }
    }

    /// The ids an invoke node's `FromNode` edges name, in argument order.
    fn upstreams(&self) -> Vec<&str> {
        match self {
            PipelineNode::Source { .. } => Vec::new(),
            PipelineNode::Invoke { args, .. } => args
                .iter()
                .filter_map(|(_, src)| match src {
                    ArgSource::FromNode(up) => Some(up.as_str()),
                    ArgSource::Literal(_) => None,
                })
                .collect(),
        }
    }
}

/// A dataflow of capability calls, its nodes in declaration order. Twin of
/// `CapabilityPipeline`.
#[derive(Debug, Clone, PartialEq)]
pub struct CapabilityPipeline {
    pub nodes: Vec<PipelineNode>,
}

/// Why a pipeline does not type-check. Twin of `PipelineError`.
#[derive(Debug, Clone, PartialEq)]
pub enum PipelineError {
    /// Two nodes share this id.
    DuplicateNode { id: String },
    /// An edge names a node the pipeline does not declare.
    UnknownNode { id: String },
    /// An invoke node names a capability the registry does not hold.
    PipelineNoSuchCapability { id: String, known: Vec<String> },
    /// An argument is refused as the capability's validation refuses it.
    PipelineArgRefused { node: String, reason: InvokeError },
    /// An edge closes a cycle; `cycle` names it from `node`.
    PipelineCycle { node: String, cycle: Vec<String> },
    /// An edge's producer output does not feed the argument's space.
    EdgeTypeMismatch {
        node: String,
        arg: String,
        producer: String,
        consumer: String,
    },
    /// An edge names a node declared later, without closing a cycle.
    PipelineForwardEdge {
        node: String,
        arg: String,
        upstream: String,
    },
    /// The node's declared output does not admit what its capability answers,
    /// or the capability answers nothing.
    PipelineOutputContradicts {
        node: String,
        capability: String,
        declared: ValueSpace,
        result: Returns,
    },
}

/// A space's family name, as an edge mismatch names it.
fn space_tag(s: &ValueSpace) -> &'static str {
    match s {
        ValueSpace::IntRange(..) => "int",
        ValueSpace::FloatRange(..) => "float",
        ValueSpace::StringLen(..) => "string",
        ValueSpace::Enum(_) => "enum",
        ValueSpace::AnyString => "anyString",
        ValueSpace::SlotTree(_) => "slotTree",
        ValueSpace::Document(_) => "document",
    }
}

fn arg_source_of(c: &Cursor<'_>) -> Result<ArgSource, DecodeError> {
    Ok(match c.tag("$type", &["literal", "fromNode"])? {
        "literal" => ArgSource::Literal(c.field("value")?.string()?.to_string()),
        _ => ArgSource::FromNode(c.field("node")?.string()?.to_string()),
    })
}

fn node_of(c: &Cursor<'_>) -> Result<PipelineNode, DecodeError> {
    let tag = c.tag("$type", &["source", "invoke"])?;
    let id = c.field("id")?.string()?.to_string();
    let output_type = space_of(&c.field("outputType")?)?;
    Ok(match tag {
        "source" => PipelineNode::Source {
            id,
            data_ref: c.field("dataRef")?.string()?.to_string(),
            output_type,
        },
        _ => PipelineNode::Invoke {
            id,
            capability_id: c.field("capabilityId")?.string()?.to_string(),
            output_type,
            args: c
                .field("args")?
                .items()?
                .iter()
                .map(|a| {
                    Ok((
                        a.field("addr")?.string()?.to_string(),
                        arg_source_of(&a.field("source")?)?,
                    ))
                })
                .collect::<Result<_, DecodeError>>()?,
        },
    })
}

impl CapabilityPipeline {
    /// Read a pipeline from its wire text: strict JSON, then the pipeline seam's
    /// declaration read leniently ([`seam::CAPABILITY_PIPELINE`], which refuses as
    /// the reference's derived codec does, an ill-formed output space included),
    /// then the typed nodes. Twin of `CapabilityPipeline.decodeDetailed`.
    pub fn decode(text: &str) -> Result<CapabilityPipeline, DecodeError> {
        let v = decode::parse(text, &JsonLimits::STRICT)?;
        let reading = Reading {
            strict: false,
            param: Slot::Str,
        };
        seam::read(&Slot::Record(&seam::CAPABILITY_PIPELINE), &v, &reading)?;
        let nodes = Cursor::root(&v)
            .field("nodes")?
            .items()?
            .iter()
            .map(node_of)
            .collect::<Result<_, _>>()?;
        Ok(CapabilityPipeline { nodes })
    }

    fn node(&self, id: &str) -> Option<(usize, &PipelineNode)> {
        self.nodes.iter().enumerate().find(|(_, n)| n.id() == id)
    }

    /// The path from `start` along `FromNode` edges to a node whose edge reaches
    /// `target`: `[start, …, x]` with `x -> target`, depth first in argument
    /// order, each node visited once. Iterative, so a long chain of edges costs
    /// heap and not stack.
    fn path_to(&self, target: &str, start: &str) -> Option<Vec<String>> {
        let ups = |id: &str| {
            self.node(id)
                .map(|(_, n)| n.upstreams())
                .unwrap_or_default()
        };
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut stack: Vec<(&str, usize)> = Vec::new();
        seen.insert(start);
        if ups(start).contains(&target) {
            return Some(vec![start.to_string()]);
        }
        stack.push((start, 0));
        while let Some(top) = stack.last_mut() {
            let (cur, next) = *top;
            top.1 += 1;
            let below = ups(cur);
            let Some(&u) = below.get(next) else {
                stack.pop();
                continue;
            };
            if !seen.insert(u) {
                continue;
            }
            if ups(u).contains(&target) {
                let mut path: Vec<String> = stack.iter().map(|(n, _)| n.to_string()).collect();
                path.push(u.to_string());
                return Some(path);
            }
            stack.push((u, 0));
        }
        None
    }

    /// Type-check the pipeline against a registry: node ids are unique; every
    /// invoke's capability resolves (default-deny); its declared output admits
    /// the capability's declared result (`PipelineOutputContradicts`, before any
    /// argument is read); every argument is one the capability takes, once
    /// (`DuplicateArg`), refused as [`Capability::validate_args`] refuses it; a
    /// `FromNode` edge names a declared node EARLIER in declaration order (a
    /// self-edge or an edge closing a cycle is `PipelineCycle`, any other later
    /// node `PipelineForwardEdge`) whose output space feeds the argument's
    /// ([`ValueSpace::subsumes`], else `EdgeTypeMismatch`); every required hole is
    /// bound. The first refusal in declaration order is the answer. The registry's
    /// gates are not consulted: typing is not deciding. Twin of
    /// `CapabilityPipeline.typeCheck`.
    pub fn type_check<C>(&self, registry: &CapabilityRegistry<C>) -> Result<(), PipelineError> {
        // The first id, in first-occurrence order, that two nodes share.
        let ids: Vec<&str> = self.nodes.iter().map(PipelineNode::id).collect();
        if let Some(dup) = ids
            .iter()
            .find(|id| ids.iter().filter(|x| x == id).count() > 1)
        {
            return Err(PipelineError::DuplicateNode {
                id: dup.to_string(),
            });
        }
        for (position, n) in self.nodes.iter().enumerate() {
            let PipelineNode::Invoke {
                id: nid,
                capability_id,
                output_type,
                args,
            } = n
            else {
                continue;
            };
            let Some(cap) = registry.try_find(capability_id) else {
                return Err(PipelineError::PipelineNoSuchCapability {
                    id: capability_id.clone(),
                    known: registry.ids(),
                });
            };
            let states = match &cap.result {
                Returns::Value(answered) => output_type.subsumes(answered),
                Returns::Nothing => false,
            };
            if !states {
                return Err(PipelineError::PipelineOutputContradicts {
                    node: nid.clone(),
                    capability: capability_id.clone(),
                    declared: output_type.clone(),
                    result: cap.result.clone(),
                });
            }
            self.check_args(position, nid, cap, args)?;
        }
        Ok(())
    }

    fn check_args(
        &self,
        position: usize,
        nid: &str,
        cap: &Capability,
        args: &[(String, ArgSource)],
    ) -> Result<(), PipelineError> {
        let refused = |reason| PipelineError::PipelineArgRefused {
            node: nid.to_string(),
            reason,
        };
        if let Some(addr) = repeated_addrs(args).into_iter().next() {
            return Err(refused(InvokeError::DuplicateArg { addr }));
        }
        for (addr, src) in args {
            let fault = match src {
                ArgSource::Literal(v) => cap.arg_fault(addr, v).map(refused),
                ArgSource::FromNode(up) => match cap.hole(addr) {
                    None => Some(refused(InvokeError::UnknownArg {
                        addr: addr.clone(),
                        declared: cap.declared(),
                    })),
                    Some(h) => match h.space() {
                        None => Some(refused(InvokeError::UninvocableArg { addr: addr.clone() })),
                        Some(arg_space) => self.edge_fault(position, nid, addr, up, &arg_space),
                    },
                },
            };
            if let Some(e) = fault {
                return Err(e);
            }
        }
        let unbound = cap.unbound(args);
        if unbound.is_empty() {
            Ok(())
        } else {
            Err(refused(InvokeError::RequiredArgsUnbound { addrs: unbound }))
        }
    }

    fn edge_fault(
        &self,
        position: usize,
        nid: &str,
        addr: &str,
        up: &str,
        arg_space: &ValueSpace,
    ) -> Option<PipelineError> {
        let Some((up_position, up_node)) = self.node(up) else {
            return Some(PipelineError::UnknownNode { id: up.to_string() });
        };
        if up == nid {
            return Some(PipelineError::PipelineCycle {
                node: nid.to_string(),
                cycle: vec![nid.to_string()],
            });
        }
        if up_position > position {
            return Some(match self.path_to(nid, up) {
                Some(path) => {
                    let mut cycle = vec![nid.to_string()];
                    cycle.extend(path);
                    PipelineError::PipelineCycle {
                        node: nid.to_string(),
                        cycle,
                    }
                }
                None => PipelineError::PipelineForwardEdge {
                    node: nid.to_string(),
                    arg: addr.to_string(),
                    upstream: up.to_string(),
                },
            });
        }
        if arg_space.subsumes(up_node.output_type()) {
            None
        } else {
            Some(PipelineError::EdgeTypeMismatch {
                node: nid.to_string(),
                arg: addr.to_string(),
                producer: space_tag(up_node.output_type()).to_string(),
                consumer: space_tag(arg_space).to_string(),
            })
        }
    }
}
