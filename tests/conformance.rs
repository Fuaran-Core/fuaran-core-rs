//! Certifies the spine against the conformance vectors the reference emits, read
//! from a checkout of Core (`Fuaran-Core/fuaran-core`) at the commit
//! `core-pin.json` names.
//!
//! The checkout is found at `FUARAN_CORE_DIR`, or as `fuaran-core/` beside this
//! repository or beside any directory above it (the layout CI checks it out in).
//! It is NOT optional: a suite that skipped its vectors would read green while
//! certifying nothing, so an absent checkout fails every leg here, naming how to
//! provide it.
//!
//! Each leg asserts the vector count its family's `manifest.json` declares (where
//! the family has one), and fails on a vector it did not run, so a leg that
//! silently ran nothing cannot pass.

mod common;

use common::*;
use fuaran_core::chain::{self, Actor, Anchor, ChainRecord};
use fuaran_core::dag::{Dag, DagNode, ReplayAllFault, ReplayFault};
use fuaran_core::decimal;
use fuaran_core::decode::{self, Cursor, DecodeCode, DecodeError};
use fuaran_core::json::{
    self, JVal, JsonLimits, JsonRefusal, PathSeg, escape_string, render_canonical,
};
use fuaran_core::number::format_finite_double;
use fuaran_core::ops::{self, Op};
use fuaran_core::tree::Node;

fn actor_of(text: &str) -> Actor {
    let v = json::parse(text).expect("an actor is JSON");
    let actor = match s(&v, "kind") {
        "human" => Actor::Human {
            id: s(&v, "id").to_string(),
        },
        "agent" => Actor::Agent {
            model: s(&v, "model").to_string(),
            version: s(&v, "version").to_string(),
            id: s(&v, "id").to_string(),
        },
        other => panic!("unknown actor kind {other}"),
    };
    assert_eq!(
        chain::encode_actor(&actor),
        text,
        "the actor's canonical text"
    );
    actor
}

fn tree_of(text: &str) -> Node {
    let v = decode::parse(text, &JsonLimits::STRICT).expect("a tree is JSON");
    let t = Node::decode(&Cursor::root(&v)).expect("a tree decodes");
    assert_eq!(t.encode(), text, "a tree's canonical bytes");
    t
}

fn op_of(text: &str) -> Op {
    let op = Op::parse(text).expect("an op decodes");
    assert_eq!(op.encode(), text, "an op's canonical bytes");
    op
}

fn hash_of(bytes: &str) -> String {
    fuaran_core::address::of_bytes(bytes)
}

// ─── refusals/codec-refusals.json — the JSON reader ──────────────────────────

#[test]
fn codec_refusals_json_vectors() {
    let doc = read("refusals/codec-refusals.json");
    assert_eq!(vectors(&doc).len(), declared("refusals", "codecRefusals"));
    let mut ran = 0;
    for v in vectors(&doc).iter().filter(|v| s(v, "codec") == "json") {
        let name = s(v, "name");
        let got = json::parse(s(v, "input"));
        match (v.field("refusal"), got) {
            (Some(JVal::Str(class)), Err(JsonRefusal::Malformed(e))) => {
                assert_eq!(e.kind.as_str(), class, "{name}");
            }
            (None, Ok(value)) => assert_eq!(render_canonical(&value), s(v, "canonical"), "{name}"),
            (want, got) => panic!("{name}: expected {want:?}, got {got:?}"),
        }
        ran += 1;
    }
    // The column vectors run in tests/columnar.rs (26) and the profile vectors in
    // tests/propagation_forms.rs (19): the three legs are the family's 101.
    assert_eq!(ran, 56, "the json vectors run");
}

// ─── decode/decode-rejects.json — the decode error contract ──────────────────

fn str_of(v: &JVal, k: &str) -> Option<String> {
    match v.field(k) {
        Some(JVal::Str(x)) => Some(x.clone()),
        _ => None,
    }
}

/// The vector's shape grammar, read with the spine's combinators exactly as the
/// reference's shape decoder reads it.
fn read_shape(defs: &JVal, shape: &JVal, at: &Cursor<'_>) -> Result<(), DecodeError> {
    let any = JVal::Obj(vec![("$type".into(), JVal::Str("any".into()))]);
    match s(shape, "$type") {
        "string" => at.string().map(|_| ()),
        "int" => at.int().map(|_| ()),
        "float" => at.float().map(|_| ()),
        "bool" => at.boolean().map(|_| ()),
        "any" => Ok(()),
        "list" => {
            let of = shape.field("of").unwrap_or(&any);
            let items = match shape.field("maxItems") {
                Some(JVal::Num(max)) => at.items_at_most(*max as usize)?,
                _ => at.items()?,
            };
            items.iter().try_for_each(|i| read_shape(defs, of, i))
        }
        "object" => {
            let fields = match shape.field("fields") {
                Some(JVal::Arr(fs)) => fs.as_slice(),
                _ => &[],
            };
            if shape.field("closed") == Some(&JVal::Bool(true)) {
                let names: Vec<&str> = fields.iter().map(|f| s(f, "name")).collect();
                at.closed(&names)?;
            }
            for f in fields {
                let sub = f.field("shape").unwrap_or(&any);
                if f.field("optional") == Some(&JVal::Bool(true)) {
                    if let Some(c) = at.optional(s(f, "name"))? {
                        read_shape(defs, sub, &c)?;
                    }
                } else {
                    read_shape(defs, sub, &at.field(s(f, "name"))?)?;
                }
            }
            if fields.is_empty() {
                at.optional("")?; // still an object
            }
            Ok(())
        }
        "tagged" => {
            let key = str_of(shape, "key").unwrap_or_else(|| "kind".into());
            let cases = arr(shape, "cases");
            let tags: Vec<&str> = cases.iter().map(|c| s(c, "tag")).collect();
            let tag = at.tag(&key, &tags)?;
            if let Some(JVal::Arr(admit)) = shape.field("admit") {
                if !admit.iter().any(|a| a == &JVal::Str(tag.to_string())) {
                    return Err(at.field(&key)?.fail(
                        DecodeCode::NotAdmitted,
                        format!("'{tag}' is not admitted here"),
                    ));
                }
            }
            let case = cases.iter().find(|c| s(c, "tag") == tag).unwrap();
            read_shape(defs, get(case, "shape"), at)
        }
        "intRange" => at
            .int_range(n(shape, "min") as i32, n(shape, "max") as i32)
            .map(|_| ()),
        "ref" => match defs.field(s(shape, "name")) {
            Some(def) => read_shape(defs, def, at),
            None => Err(at.fail(DecodeCode::SchemaFault, "an unresolved shape")),
        },
        other => panic!("unknown shape {other}"),
    }
}

fn limits_of(v: &JVal) -> JsonLimits {
    match v.field("limits") {
        None => JsonLimits::STRICT,
        Some(l) => JsonLimits {
            depth: n(l, "depth") as usize,
            string_length: Some(n(l, "stringLength") as usize),
            array_length: Some(n(l, "arrayLength") as usize),
            object_members: Some(n(l, "objectMembers") as usize),
            document_bytes: Some(n(l, "documentBytes") as usize),
        },
    }
}

fn path_json(path: &[PathSeg]) -> String {
    let items: Vec<String> = path
        .iter()
        .map(|p| match p {
            PathSeg::Key(k) => escape_string(k),
            PathSeg::Index(i) => i.to_string(),
        })
        .collect();
    json::render_array(&items)
}

#[test]
fn decode_rejects_vectors() {
    let doc = read("decode/decode-rejects.json");
    assert_eq!(vectors(&doc).len(), declared("decode", "decodeRejects"));
    let empty = JVal::Obj(vec![]);
    let mut ran = 0;
    for v in vectors(&doc) {
        let name = s(v, "name");
        let defs = v.field("definitions").unwrap_or(&empty);
        let outcome = decode::parse(s(v, "input"), &limits_of(v))
            .and_then(|value| read_shape(defs, get(v, "shape"), &Cursor::root(&value)));
        match (v.field("refusal"), outcome) {
            (None, Ok(())) => assert_eq!(v.field("accepted"), Some(&JVal::Bool(true)), "{name}"),
            (Some(r), Err(e)) => {
                assert_eq!(e.code.as_str(), s(r, "code"), "{name}: {}", e.message);
                assert_eq!(
                    path_json(&e.path),
                    render_canonical(get(r, "path")),
                    "{name}"
                );
            }
            (want, got) => panic!("{name}: expected {want:?}, got {got:?}"),
        }
        ran += 1;
    }
    assert_eq!(ran, 47);
}

// ─── laws/decimal-laws.json — the exact decimal ──────────────────────────────

#[test]
fn decimal_law_vectors() {
    let doc = read("laws/decimal-laws.json");
    let mut ran = 0;
    for v in vectors(&doc) {
        let (id, input, expected) = (s(v, "id"), get(v, "input"), get(v, "expected"));
        let accept = s(expected, "verdict") == "accept";
        let answer: Option<String> = match s(v, "case") {
            "canonical" => decimal::try_canonical(s(input, "text")),
            "compare" => {
                decimal::compare(s(input, "a"), s(input, "b")).map(|o| (o as i8).to_string())
            }
            "add" => decimal::add(s(input, "a"), s(input, "b")),
            "toFloat" => decimal::try_to_float(s(input, "text")).map(format_finite_double),
            // The column codec and the aggregates run in tests/columnar.rs (22):
            // the two legs are the family's 75.
            _ => continue,
        };
        let want = match (s(v, "case"), accept) {
            (_, false) => None,
            ("canonical", true) => Some(s(expected, "canonical").to_string()),
            ("compare", true) => Some((n(expected, "order") as i8).to_string()),
            ("add", true) => Some(s(expected, "sum").to_string()),
            ("toFloat", true) => Some(s(expected, "float").to_string()),
            _ => unreachable!(),
        };
        assert_eq!(answer, want, "{id}");
        ran += 1;
    }
    assert_eq!(
        ran, 53,
        "the canonical, compare, add and toFloat vectors run"
    );
    assert_eq!(vectors(&doc).len(), declared("laws", "decimal"));
}

// ─── escape/string-escape.json — the one string spelling ─────────────────────

#[test]
fn string_escape_lines() {
    let doc = read("escape/string-escape.json");
    let mut ran = 0;
    for line in arr(&doc, "lines") {
        let JVal::Str(line) = line else {
            panic!("a line is a string")
        };
        let fields: Vec<&str> = line.split('\t').collect();
        assert_eq!(fields.len(), 3, "{line}");
        if fields[2].starts_with("{\"seq\"") {
            let actor = actor_of(fields[1]);
            assert_eq!(
                chain::payload(0, &chain::encode_actor(&actor), "{}"),
                fields[2]
            );
        } else {
            let ch = match fields[0].strip_prefix("U+") {
                Some(hex) => char::from_u32(u32::from_str_radix(hex, 16).unwrap())
                    .unwrap()
                    .to_string(),
                None => {
                    let JVal::Str(id) = get(&json::parse(fields[2]).unwrap(), "id").clone() else {
                        panic!()
                    };
                    id
                }
            };
            assert_eq!(
                escape_string(&ch),
                format!("\"{}\"", fields[1]),
                "{}",
                fields[0]
            );
            let actor = Actor::Human { id: ch };
            assert_eq!(chain::encode_actor(&actor), fields[2], "{}", fields[0]);
        }
        ran += 1;
    }
    assert_eq!(ran, 38);
}

// ─── apply/skeleton-apply.json — the skeleton ops ────────────────────────────

#[test]
fn skeleton_apply_vectors() {
    let doc = read("apply/skeleton-apply.json");
    assert_eq!(vectors(&doc).len(), declared("apply", "skeletonApply"));
    for v in vectors(&doc) {
        let (id, input, expected) = (s(v, "id"), get(v, "input"), get(v, "expected"));
        let tree = tree_of(s(input, "tree"));
        let op = op_of(s(input, "op"));
        match (s(expected, "verdict"), ops::apply(&tree, &op)) {
            ("accept", Ok(t)) => {
                let bytes = t.encode();
                assert_eq!(bytes, s(expected, "tree"), "{id}");
                assert_eq!(hash_of(&bytes), s(expected, "hash"), "{id}");
            }
            ("reject", Err(r)) => assert_eq!(r.encode(), s(expected, "rejection"), "{id}"),
            (want, got) => panic!("{id}: expected {want}, got {got:?}"),
        }
    }
}

// ─── chain/skeleton-chain.json — the op-stream chain ─────────────────────────

fn anchor_of(v: &JVal) -> Anchor {
    Anchor {
        prev_hash: s(v, "prevHash").to_string(),
        seq: n(v, "seq") as u64,
    }
}

fn assert_anchor(a: &Anchor, want: &JVal, id: &str) {
    assert_eq!(a, &anchor_of(want), "{id}: the tip");
}

#[test]
fn skeleton_chain_vectors() {
    let doc = read("chain/skeleton-chain.json");
    assert_eq!(vectors(&doc).len(), declared("chain", "skeletonChain"));
    for v in vectors(&doc) {
        let (id, input, expected) = (s(v, "id"), get(v, "input"), get(v, "expected"));
        let anchor = anchor_of(get(input, "anchor"));
        match s(v, "case") {
            "append" => {
                let mut at = anchor;
                let want = arr(expected, "records");
                let entries = arr(input, "entries");
                assert_eq!(entries.len(), want.len(), "{id}");
                for (e, w) in entries.iter().zip(want) {
                    let (r, next) = chain::append(&at, s(e, "actor"), s(e, "op"));
                    assert_eq!(r.seq, n(w, "seq") as u64, "{id}");
                    assert_eq!(r.prev_hash, s(w, "prevHash"), "{id}");
                    assert_eq!(r.payload(), s(w, "payload"), "{id}");
                    assert_eq!(r.hash, s(w, "hash"), "{id}");
                    at = next;
                }
                assert_anchor(&at, get(expected, "tip"), id);
            }
            "walk" => {
                let records: Vec<ChainRecord> = arr(input, "records")
                    .iter()
                    .map(|r| ChainRecord {
                        seq: n(r, "seq") as u64,
                        actor: s(r, "actor").to_string(),
                        op: s(r, "op").to_string(),
                        prev_hash: s(r, "prevHash").to_string(),
                        hash: s(r, "hash").to_string(),
                    })
                    .collect();
                match (s(expected, "verdict"), chain::walk(&anchor, &records)) {
                    ("intact", Ok(tip)) => assert_anchor(&tip, get(expected, "tip"), id),
                    ("break", Err(b)) => {
                        assert_eq!(b.index, n(expected, "index") as usize, "{id}");
                        assert_eq!(b.reason.as_str(), s(expected, "reason"), "{id}");
                        assert_eq!(b.expected, s(expected, "expected"), "{id}");
                        assert_eq!(b.got, s(expected, "got"), "{id}");
                    }
                    (want, got) => panic!("{id}: expected {want}, got {got:?}"),
                }
            }
            other => panic!("{id}: unknown case {other}"),
        }
    }
}

// ─── dag/skeleton-dag.json — the op-DAG ──────────────────────────────────────

fn apply_tree(op: &Op, t: Node) -> Result<Node, ops::Rejection> {
    ops::apply(&t, op)
}

#[test]
fn skeleton_dag_vectors() {
    let doc = read("dag/skeleton-dag.json");
    assert_eq!(vectors(&doc).len(), declared("dag", "skeletonDag"));
    for v in vectors(&doc) {
        let (id, input, expected) = (s(v, "id"), get(v, "input"), get(v, "expected"));
        match s(v, "case") {
            "build" => check_build(id, input, expected),
            "verify" => {
                let nodes = arr(input, "nodes")
                    .iter()
                    .map(|x| DagNode {
                        id: s(x, "id").to_string(),
                        parents: str_items(x, "parents"),
                        actor: actor_of(s(x, "actor")),
                        op: op_of(s(x, "op")),
                    })
                    .collect();
                let dag = Dag::from_nodes(nodes);
                match (s(expected, "verdict"), dag.first_break()) {
                    ("intact", None) => {}
                    ("break", Some(b)) => {
                        assert_eq!(b.node, s(expected, "node"), "{id}");
                        assert_eq!(b.reason.as_str(), s(expected, "reason"), "{id}");
                        assert_eq!(b.expected, s(expected, "expected"), "{id}");
                        assert_eq!(b.got, s(expected, "got"), "{id}");
                    }
                    (want, got) => panic!("{id}: expected {want}, got {got:?}"),
                }
            }
            other => panic!("{id}: unknown case {other}"),
        }
    }
}

fn check_build(id: &str, input: &JVal, expected: &JVal) {
    let state0 = tree_of(s(input, "state0"));
    let mut dag: Dag<Op> = Dag::new();
    // label -> id, the first write to record a node names it.
    let mut ids: Vec<(String, String)> = Vec::new();
    let mut names: Vec<(String, String)> = Vec::new();
    let writes = arr(input, "writes");
    let want_writes = arr(expected, "writes");
    assert_eq!(writes.len(), want_writes.len(), "{id}");
    for (w, want) in writes.iter().zip(want_writes) {
        let label = s(w, "label");
        let parents: Vec<String> = str_items(w, "parents")
            .into_iter()
            .map(|p| match p.strip_prefix('@') {
                Some(l) => ids.iter().find(|(x, _)| x == l).unwrap().1.clone(),
                None => p,
            })
            .collect();
        let (actor, op) = (actor_of(s(w, "actor")), op_of(s(w, "op")));
        let written = match s(w, "verb") {
            "appendOn" => dag.append_on(actor, op, parents),
            "mergeAll" => dag.merge_all(actor, op, parents),
            other => panic!("{id}: unknown verb {other}"),
        };
        assert_eq!(s(want, "label"), label, "{id}");
        match written {
            Ok(node) => {
                assert_eq!(node, s(want, "id"), "{id}: write {label}");
                ids.push((label.to_string(), node.clone()));
                if !names.iter().any(|(n, _)| *n == node) {
                    names.push((node, label.to_string()));
                }
            }
            Err(f) => assert_eq!(f.encode(), s(want, "refused"), "{id}: write {label}"),
        }
    }
    let name = |node: &str| -> String {
        names
            .iter()
            .find(|(n, _)| n == node)
            .map(|(_, l)| l.clone())
            .unwrap_or_else(|| panic!("{id}: no label for {node}"))
    };
    let heads: Vec<String> = dag.heads().iter().map(|h| name(h)).collect();
    assert_eq!(heads, str_items(expected, "heads"), "{id}: heads");
    let order: Vec<String> = dag.total_order().unwrap().iter().map(|h| name(h)).collect();
    assert_eq!(order, str_items(expected, "order"), "{id}: order");
    let ranks = dag.ranks().unwrap();
    let mut got_ranks: Vec<(String, usize)> = ranks.iter().map(|(k, r)| (name(k), *r)).collect();
    got_ranks.sort();
    let JVal::Obj(want_ranks) = get(expected, "ranks") else {
        panic!()
    };
    let mut want_ranks: Vec<(String, usize)> = want_ranks
        .iter()
        .map(|(k, r)| match r {
            JVal::Num(x) => (k.clone(), *x as usize),
            _ => panic!(),
        })
        .collect();
    want_ranks.sort();
    assert_eq!(got_ranks, want_ranks, "{id}: ranks");

    let replays = arr(expected, "replays");
    assert_eq!(replays.len(), dag.heads().len(), "{id}");
    for (head, want) in dag.heads().iter().zip(replays) {
        assert_eq!(name(head), s(want, "head"), "{id}");
        let got = dag.replay_to(state0.clone(), head, apply_tree);
        match (s(want, "verdict"), got) {
            ("accept", Ok(t)) => {
                assert_eq!(t.encode(), s(want, "tree"), "{id}: replay {head}");
                assert_eq!(hash_of(&t.encode()), s(want, "hash"), "{id}");
            }
            ("reject", Err(ReplayFault::Rejected(node, r))) => {
                assert_eq!(name(&node), s(want, "node"), "{id}");
                assert_eq!(r.encode(), s(want, "rejection"), "{id}");
            }
            (w, g) => panic!("{id}: replay {head}: expected {w}, got {g:?}"),
        }
    }
    let all = get(expected, "replayAll");
    match (s(all, "verdict"), dag.replay_all(state0, apply_tree)) {
        ("accept", Ok(t)) => {
            assert_eq!(t.encode(), s(all, "tree"), "{id}: replayAll");
            assert_eq!(hash_of(&t.encode()), s(all, "hash"), "{id}");
        }
        ("reject", Err(ReplayAllFault::Rejected(node, r))) => {
            assert_eq!(name(&node), s(all, "node"), "{id}");
            assert_eq!(r.encode(), s(all, "rejection"), "{id}");
        }
        (w, g) => panic!("{id}: replayAll: expected {w}, got {g:?}"),
    }
}
