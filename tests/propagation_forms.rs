//! Conformance legs for propagation, the canonical-form declarations and the wire
//! profile: `laws/propagation-laws.json`, `laws/canonical-form-laws.json`, and the
//! `profile` vectors of `refusals/codec-refusals.json`.

mod common;
use common::*;

use std::cell::RefCell;

use fuaran_core::canonical_form::{self, Vocabulary};
use fuaran_core::json::{JVal, ordinal_cmp};
use fuaran_core::profile;
use fuaran_core::propagation::{
    self, Deps, EvalOutcome, PropagationError, Resolve, dirty_from_changed_ids, sort, valid_topo,
};

// ─── laws/propagation-laws.json ──────────────────────────────────────────────

fn deps_of(v: &JVal) -> Deps {
    match get(v, "deps") {
        JVal::Obj(m) => m
            .iter()
            .map(|(k, reads)| match reads {
                JVal::Arr(xs) => (
                    k.clone(),
                    xs.iter()
                        .map(|x| match x {
                            JVal::Str(s) => s.clone(),
                            other => panic!("a read is not a string: {other:?}"),
                        })
                        .collect(),
                ),
                other => panic!("reads are not an array: {other:?}"),
            })
            .collect(),
        other => panic!("deps is not an object: {other:?}"),
    }
}

fn values_of(v: &JVal, k: &str) -> std::collections::BTreeMap<String, i64> {
    match get(v, k) {
        JVal::Obj(m) => m
            .iter()
            .map(|(k, x)| match x {
                JVal::Num(n) => (k.clone(), *n as i64),
                other => panic!("a value is not a number: {other:?}"),
            })
            .collect(),
        other => panic!("'{k}' is not an object: {other:?}"),
    }
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort_by(|a, b| ordinal_cmp(a, b));
    ids
}

/// The toy evaluator the vectors are computed with: `value(n) = base(n) + Σ
/// value(r)` over n's declared reads, a read resolving to nothing counting 0;
/// `leak` makes one node also read an id it never declared. Records each node it
/// is invoked on.
fn evaluator<'a>(
    base: &'a std::collections::BTreeMap<String, i64>,
    deps: &'a Deps,
    leak: Option<(&'a str, &'a str)>,
    invoked: &'a RefCell<Vec<String>>,
) -> impl FnMut(&mut Resolve<'_, i64>, &str) -> Result<i64, String> + 'a {
    move |resolve, id| {
        invoked.borrow_mut().push(id.to_string());
        let mut sum = base[id];
        for r in &deps[id] {
            sum += resolve(r).unwrap_or(0);
        }
        if let Some((at, r)) = leak {
            if at == id {
                sum += resolve(r).unwrap_or(0);
            }
        }
        Ok(sum)
    }
}

/// A driver's answer in the vector's words.
fn outcome(r: Result<EvalOutcome<i64>, PropagationError>) -> String {
    let row = |fields: Vec<(&str, String)>| {
        let mut f: Vec<(String, String)> = fields
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        fuaran_core::json::render_object(&mut f)
    };
    let ids = |xs: &[String]| {
        let r: Vec<String> = xs
            .iter()
            .map(|x| fuaran_core::json::escape_string(x))
            .collect();
        fuaran_core::json::render_array(&r)
    };
    let s = fuaran_core::json::escape_string;
    match r {
        Ok(o) => {
            let mut values: Vec<(String, String)> = o
                .values
                .iter()
                .map(|(k, v)| (k.clone(), v.to_string()))
                .collect();
            let cycles: Vec<String> = o.cyclic.iter().map(|c| ids(&sorted(c.clone()))).collect();
            row(vec![
                ("verdict", s("accept")),
                ("values", fuaran_core::json::render_object(&mut values)),
                ("cycles", fuaran_core::json::render_array(&cycles)),
            ])
        }
        Err(PropagationError::EvalUndeclaredRead { node, read }) => row(vec![
            ("verdict", s("reject")),
            ("error", s("evalUndeclaredRead")),
            ("node", s(&node)),
            ("read", s(&read)),
        ]),
        Err(PropagationError::EvalUnknownChange(unknown)) => row(vec![
            ("verdict", s("reject")),
            ("error", s("evalUnknownChange")),
            ("ids", ids(&unknown)),
        ]),
        Err(PropagationError::EvalNodeFailed { node, .. }) => row(vec![
            ("verdict", s("reject")),
            ("error", s("evalNodeFailed")),
            ("node", s(&node)),
        ]),
    }
}

fn strip(expected: &JVal, drop: &str) -> String {
    match expected {
        JVal::Obj(m) => fuaran_core::json::render_canonical(&JVal::Obj(
            m.iter().filter(|(k, _)| k != drop).cloned().collect(),
        )),
        other => panic!("expected is not an object: {other:?}"),
    }
}

#[test]
fn propagation_law_vectors() {
    let doc = read("laws/propagation-laws.json");
    // Six vectors per drawn graph, and the fixed cyclic graph's one.
    let drawn = n(&doc, "iterations") as usize * 6 + 1;
    let mut ran = 0;
    for v in vectors(&doc) {
        let (id, input, expected) = (s(v, "id"), get(v, "input"), get(v, "expected"));
        let deps = deps_of(input);
        match s(v, "case") {
            "dirtySet" => {
                let changed = str_items(input, "changed").into_iter().collect();
                let dirty = sorted(
                    dirty_from_changed_ids(&deps, &changed)
                        .into_iter()
                        .collect(),
                );
                assert_eq!(dirty, str_items(expected, "dirty"), "{id}");
            }
            "evaluationOrder" => {
                let topo = sort(&deps);
                assert!(valid_topo(&deps, &topo), "{id}: the order is not valid");
                assert_eq!(topo.order, str_items(expected, "order"), "{id}");
                let cycles: Vec<Vec<String>> = topo.cycles.into_iter().map(sorted).collect();
                let want: Vec<Vec<String>> = arr(expected, "cycles")
                    .iter()
                    .map(|c| match c {
                        JVal::Arr(xs) => xs
                            .iter()
                            .map(|x| match x {
                                JVal::Str(s) => s.clone(),
                                other => panic!("{other:?}"),
                            })
                            .collect(),
                        other => panic!("{other:?}"),
                    })
                    .collect();
                assert_eq!(cycles, want, "{id}");
            }
            "evalFrom" => {
                let (base0, base1) = (values_of(input, "base"), values_of(input, "changedBase"));
                let changed = str_items(input, "changed").into_iter().collect();
                let none = RefCell::new(Vec::new());
                let prior = propagation::eval(evaluator(&base0, &deps, None, &none), &deps)
                    .expect("the prior evaluation")
                    .values;
                let invoked = RefCell::new(Vec::new());
                let incremental = propagation::eval_from(
                    evaluator(&base1, &deps, None, &invoked),
                    &prior,
                    &changed,
                    &deps,
                );
                let accepted = incremental.is_ok();
                if accepted {
                    // The incremental answer is a full evaluation's.
                    let full = propagation::eval(evaluator(&base1, &deps, None, &none), &deps);
                    assert_eq!(incremental, full, "{id}: evalFrom is not a full eval");
                    let recomputed: std::collections::BTreeSet<String> =
                        invoked.borrow().iter().cloned().collect();
                    assert_eq!(
                        sorted(recomputed.into_iter().collect()),
                        str_items(expected, "recomputed"),
                        "{id}"
                    );
                }
                assert_eq!(outcome(incremental), strip(expected, "recomputed"), "{id}");
            }
            "undeclaredRead" => {
                let base = values_of(input, "base");
                let (node, read_id) = (s(input, "node"), s(input, "read"));
                let leak = Some((node, read_id));
                let scratch = RefCell::new(Vec::new());
                let want = render(expected);
                let full = propagation::eval(evaluator(&base, &deps, leak, &scratch), &deps);
                assert_eq!(outcome(full), want, "{id}: eval");
                // evalFrom over a prior primed without the leak, the leaking node changed.
                let prior = propagation::eval(evaluator(&base, &deps, None, &scratch), &deps)
                    .expect("the prior evaluation")
                    .values;
                let changed = [node.to_string()].into_iter().collect();
                let from = propagation::eval_from(
                    evaluator(&base, &deps, leak, &scratch),
                    &prior,
                    &changed,
                    &deps,
                );
                assert_eq!(outcome(from), want, "{id}: evalFrom");
            }
            other => panic!("{id}: unknown case {other}"),
        }
        ran += 1;
    }
    assert_eq!(
        ran,
        declared("laws", "propagationLaws"),
        "every propagation vector runs"
    );
    assert_eq!(ran, drawn, "the file's declared iterations");
}

fn render(v: &JVal) -> String {
    fuaran_core::json::render_canonical(v)
}

// ─── laws/canonical-form-laws.json ───────────────────────────────────────────

#[test]
fn canonical_form_law_vectors() {
    let doc = read("laws/canonical-form-laws.json");
    let vocab = Vocabulary::from_json(get(&doc, "vocabulary")).expect("the fixture vocabulary");
    let mut ran = 0;
    for v in vectors(&doc) {
        let id = s(v, "id");
        assert!(
            matches!(s(v, "law"), "alias" | "presence" | "default"),
            "{id}: unknown law"
        );
        let got = canonical_form::canonical(&vocab, s(v, "document"))
            .unwrap_or_else(|e| panic!("{id}: {e:?}"));
        assert_eq!(got, s(v, "expected"), "{id}");
        // The canonical form is a fixpoint.
        assert_eq!(
            canonical_form::canonical(&vocab, &got).as_deref(),
            Ok(got.as_str()),
            "{id}: not a fixpoint"
        );
        ran += 1;
    }
    assert_eq!(
        ran,
        declared("laws", "canonicalFormLaws"),
        "every canonical-form vector runs"
    );
}

// ─── refusals/codec-refusals.json — the profile vectors ──────────────────────

#[test]
fn profile_refusal_vectors() {
    let doc = read("refusals/codec-refusals.json");
    let mut ran = 0;
    for v in vectors(&doc) {
        if s(v, "codec") != "profile" {
            continue;
        }
        let (name, input) = (s(v, "name"), s(v, "input"));
        let got = profile::try_parse(input)
            .map(|p| profile::render(&p))
            .map_err(|_| "MalformedProfile");
        let want = match v.field("refusal") {
            Some(JVal::Str(r)) => Err(r.as_str()),
            _ => Ok(s(v, "canonical").to_string()),
        };
        assert_eq!(got, want, "{name}");
        if let Ok(p) = profile::try_parse(input) {
            assert_eq!(profile::try_render(&p).as_deref(), Ok(input), "{name}");
        }
        ran += 1;
    }
    assert_eq!(ran, 19, "every profile vector runs");
}
