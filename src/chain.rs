//! The op-stream chain: an append-only sequence of records, each hashing its
//! predecessor's hash with its own pre-image, so a tamper to any historical
//! record breaks the chain at that record and the walk names the first break.
//!
//! The chain reads no op: every op is a string spliced into the pre-image
//! unread, and an actor is its canonical text ([`encode_actor`]). So one chain
//! serves every op vocabulary, and a host certifies it with no op codec.
//!
//! ```text
//! payload = {"seq":<seq>,"actor":<actor>,"op":<op>}      (exactly these bytes)
//! hash    = sha256Hex(prevHash ++ "|" ++ payload)          (lower-case hex)
//! ```
//!
//! Under the canonical profile the first record hangs from [`GENESIS`], the empty
//! string, at sequence 0; a stream that keeps another genesis (a wire's own
//! fixed anchor) starts from an [`Anchor`] naming it. Certified against Core's
//! `chain/skeleton-chain.json`, and the actor and pre-image spelling against
//! `escape/string-escape.json`.

use crate::json::escape_string;
use crate::prelude::*;
use crate::sha256::sha256_hex;

/// The canonical profile's genesis: the empty previous hash.
pub const GENESIS: &str = "";

/// Who authored an op — folded into every pre-image, so attribution is
/// tamper-evident.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Actor {
    Human {
        id: String,
    },
    Agent {
        model: String,
        version: String,
        id: String,
    },
}

/// The actor's canonical text. The member order is PINNED (`kind` first, then the
/// case's members in declaration order), not sorted: the same bytes are folded
/// into a chain payload and a DAG node id, so there is one spelling to drift.
pub fn encode_actor(a: &Actor) -> String {
    match a {
        Actor::Human { id } => format!("{{\"kind\":\"human\",\"id\":{}}}", escape_string(id)),
        Actor::Agent { model, version, id } => format!(
            "{{\"kind\":\"agent\",\"model\":{},\"version\":{},\"id\":{}}}",
            escape_string(model),
            escape_string(version),
            escape_string(id)
        ),
    }
}

/// The pre-image of the record at `seq`: the actor and the op spliced verbatim.
pub fn payload(seq: u64, actor: &str, op: &str) -> String {
    format!("{{\"seq\":{seq},\"actor\":{actor},\"op\":{op}}}")
}

/// The hash a record hangs from `prev_hash` with `payload`.
pub fn link(prev_hash: &str, payload: &str) -> String {
    let mut preimage = String::with_capacity(prev_hash.len() + 1 + payload.len());
    preimage.push_str(prev_hash);
    preimage.push('|');
    preimage.push_str(payload);
    sha256_hex(&preimage)
}

/// Where the next record hangs: its sequence number and the hash it links to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Anchor {
    pub prev_hash: String,
    pub seq: u64,
}

impl Anchor {
    /// The canonical profile's start: [`GENESIS`] at sequence 0.
    pub fn genesis() -> Anchor {
        Anchor {
            prev_hash: GENESIS.to_string(),
            seq: 0,
        }
    }
}

/// One chained record. `actor` and `op` are canonical text, carried verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainRecord {
    pub seq: u64,
    pub actor: String,
    pub op: String,
    pub prev_hash: String,
    pub hash: String,
}

impl ChainRecord {
    /// The record's pre-image, recomputed from its own members.
    pub fn payload(&self) -> String {
        payload(self.seq, &self.actor, &self.op)
    }
}

/// Chain one record onto `anchor`; answers the record and the anchor after it.
pub fn append(anchor: &Anchor, actor: &str, op: &str) -> (ChainRecord, Anchor) {
    let hash = link(&anchor.prev_hash, &payload(anchor.seq, actor, op));
    let record = ChainRecord {
        seq: anchor.seq,
        actor: actor.to_string(),
        op: op.to_string(),
        prev_hash: anchor.prev_hash.clone(),
        hash: hash.clone(),
    };
    let next = Anchor {
        prev_hash: hash,
        seq: anchor.seq + 1,
    };
    (record, next)
}

/// Why a walk stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakReason {
    /// The record's sequence number is not the next one.
    SequenceMismatch,
    /// The record does not link to its predecessor's hash.
    PrevHashBroken,
    /// The record's hash does not recompute from its members.
    HashMismatch,
}

impl BreakReason {
    /// The reason's canonical spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            BreakReason::SequenceMismatch => "sequence-number mismatch",
            BreakReason::PrevHashBroken => "prev-hash link broken",
            BreakReason::HashMismatch => "hash mismatch (tampered op/actor/seq)",
        }
    }
}

/// The first break a walk finds: the record's index in the run, the reason, and
/// the value the walk expected beside the one it found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainBreak {
    pub index: usize,
    pub reason: BreakReason,
    pub expected: String,
    pub got: String,
}

/// Walk `records` from `anchor`, checking each record's sequence number, then
/// its link, then its hash; the first fault is reported. An intact run answers
/// the anchor the next record would hang from.
pub fn walk(anchor: &Anchor, records: &[ChainRecord]) -> Result<Anchor, ChainBreak> {
    let mut at = anchor.clone();
    for (index, r) in records.iter().enumerate() {
        let fault = |reason, expected: String, got: String| ChainBreak {
            index,
            reason,
            expected,
            got,
        };
        if r.seq != at.seq {
            return Err(fault(
                BreakReason::SequenceMismatch,
                at.seq.to_string(),
                r.seq.to_string(),
            ));
        }
        if r.prev_hash != at.prev_hash {
            return Err(fault(
                BreakReason::PrevHashBroken,
                at.prev_hash,
                r.prev_hash.clone(),
            ));
        }
        let recomputed = link(&r.prev_hash, &r.payload());
        if recomputed != r.hash {
            return Err(fault(BreakReason::HashMismatch, recomputed, r.hash.clone()));
        }
        at = Anchor {
            prev_hash: r.hash.clone(),
            seq: r.seq + 1,
        };
    }
    Ok(at)
}
