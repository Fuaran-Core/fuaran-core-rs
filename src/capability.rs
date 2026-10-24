//! The invocable capability: a typed, registrable declaration of compute a host
//! runs on a caller's behalf, the default-deny argument check that runs before
//! any body, the effect-identity key a non-deterministic invocation is
//! journalled and replayed under, the canonical declaration codec, and the
//! id-ordered registry an agent enumerates.
//!
//! The twin of the reference's `Capability`, `CapabilityCodec` and
//! `CapabilityRegistry` (`Fuaran.Core.Function`), together with the pieces of
//! the signature surface they stand on — the value space (`Space`,
//! `SpaceCodec`), the two-axis effect class (`Effect`, `EffectCodec`), the
//! signature entry and its well-formedness check (`Signature.validate`).
//! Certified against Core's `laws/capability-laws.json` (`tests/capability.rs`).
//!
//! The wire carries the declaration and a typed invocation, never the body:
//! the core only validates, keys and round-trips. Running the body, and
//! routing it by its [`Placement`], is the host's.

use crate::decode::{self, Cursor, DecodeCode, DecodeError, quote_all};
use crate::hash::keyed_by;
use crate::json::{JVal, JsonLimits, ordinal_cmp, render_canonical};
use crate::number::format_finite_double;
use crate::prelude::*;

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

impl ValueSpace {
    /// The one spelling a value in the space is handed on as, or `None` where the
    /// string is not in the space: an integer in its shortest decimal form, a
    /// number in the canonical number form, every other value as sent. Twin of
    /// `Space.canonical`.
    pub fn canonical(&self, s: &str) -> Option<String> {
        match self {
            ValueSpace::IntRange(lo, hi) => read_int(s)
                .filter(|v| v >= lo && v <= hi)
                .map(|v| v.to_string()),
            ValueSpace::FloatRange(lo, hi) => read_float(s)
                .filter(|v| v >= lo && v <= hi)
                .map(format_finite_double),
            ValueSpace::StringLen(lo, hi) => {
                let len = utf16_len(s);
                (len >= i64::from(*lo) && len <= i64::from(*hi)).then(|| s.to_string())
            }
            ValueSpace::Enum(xs) => xs.iter().any(|x| x == s).then(|| s.to_string()),
            ValueSpace::AnyString => Some(s.to_string()),
            ValueSpace::SlotTree(c) => match (slot_kind_of(s), c) {
                (None, _) => None,
                (Some(_), None) => Some(s.to_string()),
                (Some(k), Some(c)) => (&k == c).then(|| s.to_string()),
            },
        }
    }

    /// Is the value in the space — exactly when it has a canonical spelling there.
    /// Twin of `Space.validate`.
    pub fn validate(&self, s: &str) -> bool {
        self.canonical(s).is_some()
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

/// A registrable, invocable capability: its registry id, its signature and where
/// its body runs. Twin of `Capability` (built by `Capability.create`).
#[derive(Debug, Clone, PartialEq)]
pub struct Capability {
    /// The registry key.
    pub id: String,
    /// The arguments it takes and the effect it declares.
    pub signature: Signature,
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
    /// A registry policy denied the invocation.
    PolicyRefused {
        policy: String,
        reason: String,
        allowed: Vec<String>,
    },
    /// A registry policy requires an approval first.
    ApprovalRequired { policy: String },
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
/// well-formedness (`IllFormedCapability`). `None` admits. Twin of `Capability.admissionOf`.
pub fn admission_of(id: &str, sg: &Signature) -> Option<InvokeError> {
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
        .map(|fault| InvokeError::IllFormedCapability {
            id: id.to_string(),
            fault,
        })
}

/// Every address an argument list binds more than once, at each repeat, in list
/// order. Twin of `Capability.repeatedAddrs`.
pub fn repeated_addrs(args: &[(String, String)]) -> Vec<String> {
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

// ─── The capture key ─────────────────────────────────────────────────────────

impl Capability {
    /// Build a capability. Twin of `Capability.create`.
    pub fn new(id: impl Into<String>, signature: Signature, placement: Placement) -> Capability {
        Capability {
            id: id.into(),
            signature,
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

    /// The effect-identity key a non-deterministic invocation is journalled and
    /// replayed under: the id, `#`, and the FNV-1a of the canonical pre-image of
    /// the addr-sorted (ordinal) bindings, two fields each, every value in its
    /// canonical spelling in its hole's space where it lies there. Twin of
    /// `Capability.invocationKey`.
    pub fn invocation_key(&self, args: &[(String, String)]) -> String {
        let mut spelled: Vec<(&str, String)> = args
            .iter()
            .map(|(a, v)| {
                let canon = self
                    .hole(a)
                    .and_then(SigEntry::space)
                    .and_then(|sp| sp.canonical(v));
                (a.as_str(), canon.unwrap_or_else(|| v.clone()))
            })
            .collect();
        // Stable, as the reference's `List.sortBy` is.
        spelled.sort_by(|x, y| ordinal_cmp(x.0, y.0));
        let fields: Vec<&str> = spelled.iter().flat_map(|(a, v)| [*a, v.as_str()]).collect();
        keyed_by(&self.id, &fields)
    }

    /// The refusal one argument earns on its own. Twin of `Capability.argFault`.
    fn arg_fault(&self, declared: &[String], addr: &str, value: &str) -> Option<InvokeError> {
        let Some(h) = self.hole(addr) else {
            return Some(InvokeError::UnknownArg {
                addr: addr.to_string(),
                declared: declared.to_vec(),
            });
        };
        match h.space() {
            None => Some(InvokeError::UninvocableArg {
                addr: addr.to_string(),
            }),
            Some(ValueSpace::SlotTree(_)) if slot_kind_of(value).is_none() => {
                Some(InvokeError::UninvocableArg {
                    addr: addr.to_string(),
                })
            }
            Some(space) if space.validate(value) => None,
            Some(space) => Some(InvokeError::ArgOutOfSpace {
                addr: addr.to_string(),
                space,
                got: value.to_string(),
            }),
        }
    }

    fn unbound(&self, args: &[(String, String)]) -> Vec<String> {
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
    /// bound (`RequiredArgsUnbound`). The first refusal wins. Twin of
    /// `Capability.validateArgs`.
    pub fn validate_args(&self, args: &[(String, String)]) -> Result<(), InvokeError> {
        if let Some(dup) = repeated_addrs(args).into_iter().next() {
            return Err(InvokeError::DuplicateArg { addr: dup });
        }
        let declared: Vec<String> = self
            .signature
            .holes
            .iter()
            .map(|h| h.addr.clone())
            .collect();
        if let Some(e) = args
            .iter()
            .find_map(|(a, v)| self.arg_fault(&declared, a, v))
        {
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
        let declared: Vec<String> = self
            .signature
            .holes
            .iter()
            .map(|h| h.addr.clone())
            .collect();
        let mut all: Vec<InvokeError> = repeated_addrs(args)
            .into_iter()
            .map(|addr| InvokeError::DuplicateArg { addr })
            .collect();
        all.extend(
            args.iter()
                .filter_map(|(a, v)| self.arg_fault(&declared, a, v)),
        );
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
/// `slotKind`). Twin of `SpaceCodec.toJson`.
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

    /// The guarded encode: refuses a declaration with a non-finite bound rather
    /// than write bytes the reader refuses; `Ok` is exactly [`Capability::encode`].
    /// Twin of `CapabilityCodec.tryEncode`.
    pub fn try_encode(&self) -> Result<String, String> {
        for e in &self.signature.holes {
            if let Some(ValueSpace::FloatRange(lo, hi)) = e.space() {
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
    /// registry's sentence); and its wire `determinism` must equal the label its
    /// signature's effect derives (`OutOfRange` at `determinism`). Twin of
    /// `CapabilityCodec.decodeJson`.
    pub fn decode_value(c: &Cursor<'_>) -> Result<Capability, DecodeError> {
        c.tag("$type", &["capability"])?;
        let id = c.field("id")?.string()?.to_string();
        let sig_at = c.field("signature")?;
        let signature = signature_of(&sig_at)?;
        if let Some(e) = admission_of(&id, &signature) {
            return Err(sig_at.field("holes")?.fail(
                DecodeCode::OutOfRange,
                format!("a declaration the registries admit: {}", e.describe()),
            ));
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

/// Read a value space from its wire document, dispatched on `"$type"`. Twin of
/// `SpaceCodec.decoder`.
pub fn space_of(c: &Cursor<'_>) -> Result<ValueSpace, DecodeError> {
    let tag = c.tag(
        "$type",
        &[
            "intRange",
            "floatRange",
            "stringLen",
            "enum",
            "anyString",
            "slotTree",
        ],
    )?;
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
        _ => ValueSpace::SlotTree(match c.optional("slotKind")? {
            Some(k) => Some(k.string()?.to_string()),
            None => None,
        }),
    })
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
    let slot = match c.optional("slotKind")? {
        Some(k) => Some(k.string()?.to_string()),
        None => None,
    };
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

// ─── The registry ────────────────────────────────────────────────────────────

/// A typed capability registry: the discovery surface an agent enumerates,
/// default-deny on dispatch. Opaque: [`CapabilityRegistry::register`] and its
/// admission gate are the only way a capability gets in. Twin of
/// `CapabilityRegistry` (without its policy gates).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CapabilityRegistry {
    /// Held in ordinal id order, the order `enumerate` answers in.
    entries: Vec<Capability>,
}

impl CapabilityRegistry {
    /// The registry with no capabilities. Twin of `CapabilityRegistry.empty`.
    pub fn new() -> CapabilityRegistry {
        CapabilityRegistry::default()
    }

    fn position(&self, id: &str) -> Result<usize, usize> {
        self.entries.binary_search_by(|c| ordinal_cmp(&c.id, id))
    }

    /// Register a capability: a held id is `DuplicateCapability` (checked first),
    /// and the admission gate's refusal otherwise; the registry is unchanged on a
    /// refusal. Twin of `CapabilityRegistry.register`.
    pub fn register(&self, c: Capability) -> Result<CapabilityRegistry, InvokeError> {
        match self.position(&c.id) {
            Ok(_) => Err(InvokeError::DuplicateCapability { id: c.id }),
            Err(at) => {
                if let Some(e) = admission_of(&c.id, &c.signature) {
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
    pub fn unregister(&self, id: &str) -> Result<CapabilityRegistry, InvokeError> {
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

    /// Resolve `id` and validate `args` — the default-deny half of dispatch that
    /// runs before any body: an id not held is `NoSuchCapability`, and the
    /// arguments are held to [`Capability::validate_args`].
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
}
