//! Certifies the read's identity and its grant against Core's
//! `laws/query-grant-laws.json` (family `queryGrantLaws`, the reference's D261):
//! the identity a grant for a read is bound to, and whether a presented grant
//! covers a read.
//!
//! The queries are read from the FILE's head (`registry`, each a canonical
//! declaration as a JSON string) through the crate's declaration reader, which
//! runs the admission gate, and each is asserted to write back the bytes it was
//! read from. The family's size is its row in `laws/manifest.json`, and the leg
//! asserts the count of each case it ran.
//!
//! The identity and a grant's coverage are answered in the forms a decision
//! uses, which build nothing (DECISIONS.md D12, D14): the identity written
//! through a writer, and the grant compared with it as it is written; each is
//! asserted to agree with the built form, the identity's `String` and
//! `Grant::covers`. A `grantCovers` vector is also decided by the query
//! registry under a gate that asks for an approval, through the one function
//! the capability registry decides by (D14): the decision is `Allow` exactly
//! where the grant covers the read. A `pageToken` is in neither identity, so the
//! read is decided without it (D261.4).

mod common;

use common::*;
use fuaran_core::capability::{Grant, PolicyDecision, PolicyGate};
use fuaran_core::column::Cell;
use fuaran_core::json::{JVal, render_canonical};
use fuaran_core::query::{Query, QueryRegistry};

const FILE: &str = "laws/query-grant-laws.json";
const CASES: [&str; 2] = ["invocationIdentity", "grantCovers"];

fn doc() -> JVal {
    let d = read(FILE);
    assert_eq!(s(&d, "family"), "queryGrantLaws");
    d
}

/// The head's queries, each read through the declaration reader and asserted
/// to write back the canonical bytes it was read from.
fn queries(d: &JVal) -> Vec<Query> {
    arr(d, "registry")
        .iter()
        .map(|t| {
            let JVal::Str(text) = t else {
                panic!("a registry entry is not a string: {t:?}")
            };
            let q = Query::decode(text)
                .unwrap_or_else(|e| panic!("the head's declaration is refused: {e:?}\n{text}"));
            assert_eq!(
                &q.encode(),
                text,
                "{}: the declaration's canonical text",
                q.id
            );
            q
        })
        .collect()
}

fn grant_of(input: &JVal) -> Option<Grant> {
    input.field("grant").map(|g| match g {
        JVal::Str(identity) => Grant::of_invocation(identity.clone()),
        other => panic!("`grant` is not a string: {other:?}"),
    })
}

/// The registry of the head's queries under one gate that asks for an approval
/// of every read.
fn registry_of(qs: &[Query]) -> QueryRegistry {
    let mut r = QueryRegistry::new();
    for q in qs {
        r = r
            .register(q.clone())
            .unwrap_or_else(|e| panic!("{}: refused at register: {e:?}", q.id));
    }
    r.with_gate(PolicyGate::ignoring_context(
        "approval",
        |_: &Query, _: &[(String, Cell)]| PolicyDecision::NeedsApproval,
    ))
}

/// The identity as a decision writes it: through a writer, building nothing but
/// the string it is written into here.
fn written_identity(q: &Query, args: &[(String, Cell)]) -> String {
    let mut out = String::new();
    q.write_invocation_identity(args, &mut out)
        .expect("a String accepts every write");
    out
}

fn answer(qs: &[Query], r: &QueryRegistry, v: &JVal) -> JVal {
    let input = get(v, "input");
    let id = s(input, "query");
    let q = qs
        .iter()
        .find(|q| q.id == id)
        .unwrap_or_else(|| panic!("{}: no query {id} in the head", s(v, "id")));
    let args = args_of(input);
    match s(v, "case") {
        "invocationIdentity" => {
            let identity = q.invocation_identity(&args);
            assert_eq!(written_identity(q, &args), identity, "{}", s(v, "id"));
            assert!(
                q.grant(&args).covers_read(q, &args),
                "{}: a read's own grant covers it",
                s(v, "id")
            );
            JVal::Obj(vec![("identity".into(), JVal::Str(identity))])
        }
        "grantCovers" => {
            let grant = grant_of(input);
            let covers = grant.as_ref().is_some_and(|g| g.covers_read(q, &args));
            let built = grant
                .as_ref()
                .is_some_and(|g| g.covers(&q.invocation_identity(&args)));
            assert_eq!(covers, built, "{}: written and built coverage", s(v, "id"));
            let decided = r
                .decide_named(&(), grant.as_ref(), id, &args)
                .unwrap_or_else(|e| panic!("{}: refused before the gate: {e:?}", s(v, "id")));
            let expected = if covers {
                PolicyDecision::Allow
            } else {
                PolicyDecision::NeedsApproval
            };
            assert_eq!(decided, ("approval", expected), "{}", s(v, "id"));
            JVal::Obj(vec![("covers".into(), JVal::Bool(covers))])
        }
        other => panic!("{}: unknown case {other}", s(v, "id")),
    }
}

#[test]
fn every_vector_of_the_family() {
    let d = doc();
    let qs = queries(&d);
    let r = registry_of(&qs);
    let mut counts = [0usize; CASES.len()];
    for v in vectors(&d) {
        let got = answer(&qs, &r, v);
        assert_eq!(
            render_canonical(&got),
            render_canonical(get(v, "expected")),
            "{}",
            s(v, "id")
        );
        let case = s(v, "case");
        counts[CASES
            .iter()
            .position(|c| *c == case)
            .expect("a case the leg answers")] += 1;
    }
    let total: usize = counts.iter().sum();
    assert_eq!(total, declared("laws", "queryGrantLaws"));
    assert_eq!(counts, [11, 8], "the vectors of each case, in {CASES:?}");
}
