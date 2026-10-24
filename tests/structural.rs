//! Certifies the spine's reconcile and diff against Core's two structural
//! families over the skeleton witness: `merge/skeleton-merge.json` (the common
//! base and the N-lane reconcile) and `diff/skeleton-diff.json` (the
//! content-aware diff). Each leg runs every vector of its family and asserts the
//! count its manifest declares.

mod common;

use common::*;
use fuaran_core::chain::{self, Actor};
use fuaran_core::dag::Dag;
use fuaran_core::decode::{self, Cursor};
use fuaran_core::diff;
use fuaran_core::json::{self, JVal, JsonLimits, render_canonical};
use fuaran_core::merge::{self, ReconcileFault};
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

fn apply_tree(op: &Op, t: Node) -> Result<Node, ops::Rejection> {
    ops::apply(&t, op)
}

fn encoded(ops: &[Op]) -> Vec<String> {
    ops.iter().map(Op::encode).collect()
}

// ─── merge/skeleton-merge.json — the N-lane reconcile ────────────────────────

/// A scenario built as the `dag/` family builds it: the DAG, each recorded
/// label's id in write order, and the label each id was first written under.
struct Built {
    dag: Dag<Op>,
    ids: Vec<(String, String)>,
}

impl Built {
    fn id(&self, label: &str) -> String {
        self.ids
            .iter()
            .rev()
            .find(|(l, _)| l == label)
            .unwrap_or_else(|| panic!("no write recorded {label}"))
            .1
            .clone()
    }

    fn label(&self, id: &str) -> String {
        self.ids
            .iter()
            .find(|(_, i)| i == id)
            .map(|(l, _)| l.clone())
            .unwrap_or_else(|| id.to_string())
    }
}

fn build(vid: &str, input: &JVal) -> Built {
    let mut b = Built {
        dag: Dag::new(),
        ids: Vec::new(),
    };
    for w in arr(input, "writes") {
        let label = s(w, "label");
        let parents: Vec<String> = str_items(w, "parents")
            .into_iter()
            .map(|p| match p.strip_prefix('@') {
                Some(l) => b.id(l),
                None => p,
            })
            .collect();
        let (actor, op) = (actor_of(s(w, "actor")), op_of(s(w, "op")));
        let written = match s(w, "verb") {
            "appendOn" => b.dag.append_on(actor, op, parents),
            "mergeAll" => b.dag.merge_all(actor, op, parents),
            other => panic!("{vid}: unknown verb {other}"),
        };
        let node = written.unwrap_or_else(|f| panic!("{vid}: write {label} refused: {f:?}"));
        b.ids.push((label.to_string(), node));
    }
    b
}

fn check_reconcile(vid: &str, input: &JVal, expected: &JVal) {
    let state0 = tree_of(s(input, "state0"));
    let b = build(vid, input);

    let want_ids = render_canonical(get(expected, "ids"));
    let got_ids = JVal::Obj(
        b.ids
            .iter()
            .map(|(l, i)| (l.clone(), JVal::Str(i.clone())))
            .collect(),
    );
    assert_eq!(render_canonical(&got_ids), want_ids, "{vid}: ids");

    let heads: Vec<String> = str_items(input, "heads").iter().map(|l| b.id(l)).collect();
    let common = b.dag.common_base(&heads);
    let common_label = common.as_deref().map(|c| b.label(c)).unwrap_or_default();
    assert_eq!(common_label, s(expected, "commonBase"), "{vid}: commonBase");

    let named = match input.field("base") {
        Some(JVal::Str(l)) => Some(b.id(l)),
        Some(other) => panic!("{vid}: base is not a string: {other:?}"),
        None => None,
    };
    let Some(base) = named.or(common) else {
        assert_eq!(s(expected, "verdict"), "noBase", "{vid}");
        return;
    };
    assert_eq!(b.label(&base), s(expected, "base"), "{vid}: base");
    let base_state = b
        .dag
        .replay_to(state0, &base, apply_tree)
        .unwrap_or_else(|f| panic!("{vid}: the base does not replay: {f:?}"));

    let outcome = merge::reconcile_many(
        &b.dag,
        |o: &Op| ops::footprint(core::slice::from_ref(o)),
        &base,
        base_state.clone(),
        &heads,
        apply_tree,
    );
    match (s(expected, "verdict"), outcome) {
        ("merged", Ok(script)) => {
            assert_eq!(encoded(&script), str_items(expected, "script"), "{vid}");
            let landed = script
                .iter()
                .try_fold(base_state, |t, op| ops::apply(&t, op))
                .unwrap_or_else(|r| panic!("{vid}: the script does not land: {r:?}"));
            assert_eq!(landed.encode(), s(expected, "tree"), "{vid}: tree");
            assert_eq!(hash_of(&landed.encode()), s(expected, "hash"), "{vid}");
        }
        ("interfere", Err(ReconcileFault::LanesInterfere(cs))) => {
            let want = arr(expected, "conflicts");
            assert_eq!(cs.len(), want.len(), "{vid}: conflicts");
            for (c, w) in cs.iter().zip(want) {
                assert_eq!(c.left.encode(), s(w, "left"), "{vid}");
                assert_eq!(c.right.encode(), s(w, "right"), "{vid}");
                assert_eq!(c.address, s(w, "address"), "{vid}");
                assert_eq!(c.shape.as_str(), s(w, "shape"), "{vid}");
            }
        }
        ("sharedRejected", Err(ReconcileFault::SharedHistoryRejected(node, r))) => {
            assert_eq!(b.label(&node), s(expected, "node"), "{vid}");
            assert_eq!(r.encode(), s(expected, "rejection"), "{vid}");
        }
        ("lanesRejected", Err(ReconcileFault::LanesRejected(lanes))) => {
            let want = arr(expected, "lanes");
            assert_eq!(lanes.len(), want.len(), "{vid}: lanes");
            for (l, w) in lanes.iter().zip(want) {
                assert_eq!(b.label(&l.head), s(w, "head"), "{vid}");
                assert_eq!(b.label(&l.node_id), s(w, "node"), "{vid}");
                assert_eq!(l.reject.encode(), s(w, "rejection"), "{vid}");
            }
        }
        (want, got) => panic!("{vid}: expected {want}, got {got:?}"),
    }
}

#[test]
fn skeleton_merge_vectors() {
    let doc = read("merge/skeleton-merge.json");
    let mut ran = 0;
    for v in vectors(&doc) {
        let (vid, input, expected) = (s(v, "id"), get(v, "input"), get(v, "expected"));
        match s(v, "case") {
            "reconcile" => check_reconcile(vid, input, expected),
            other => panic!("{vid}: unknown case {other}"),
        }
        ran += 1;
    }
    assert_eq!(ran, 12);
    assert_eq!(ran, declared("merge", "skeletonMerge"));
}

// ─── diff/skeleton-diff.json — the content-aware diff ────────────────────────

fn encode_node(n: &Node) -> String {
    n.encode()
}

#[test]
fn skeleton_diff_vectors() {
    let doc = read("diff/skeleton-diff.json");
    let mut ran = 0;
    for v in vectors(&doc) {
        let (vid, input, expected) = (s(v, "id"), get(v, "input"), get(v, "expected"));
        assert_eq!(s(v, "case"), "diff", "{vid}");
        let before = tree_of(s(input, "before"));
        let after = tree_of(s(input, "after"));
        let script = diff::to_ops_with(&encode_node, &before, &after);
        let changes = diff::changes(&encode_node, &before, &after);
        match (s(expected, "verdict"), script, changes) {
            ("diff", Ok(script), Ok(changes)) => {
                assert_eq!(encoded(&script), str_items(expected, "script"), "{vid}");
                let want = arr(expected, "changes");
                assert_eq!(changes.len(), want.len(), "{vid}: changes");
                for (c, w) in changes.iter().zip(want) {
                    assert_eq!(c.id, s(w, "id"), "{vid}");
                    assert_eq!(c.kind.encode(), render_canonical(get(w, "kind")), "{vid}");
                }
                // The script, applied to `before`, lands on `after`.
                let landed = script
                    .iter()
                    .try_fold(before, |t, op| ops::apply(&t, op))
                    .unwrap_or_else(|r| panic!("{vid}: the script does not apply: {r:?}"));
                assert_eq!(landed, after, "{vid}: the script lands on after");
            }
            ("refuse", Err(r), Err(r2)) => {
                assert_eq!(
                    r, r2,
                    "{vid}: the script and the classification refuse alike"
                );
                assert_eq!(r.encode(), s(expected, "rejection"), "{vid}");
            }
            (want, got, got_changes) => {
                panic!("{vid}: expected {want}, got {got:?} / {got_changes:?}")
            }
        }
        ran += 1;
    }
    assert_eq!(ran, 14);
    assert_eq!(ran, declared("diff", "skeletonDiff"));
}
