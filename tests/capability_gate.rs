//! Certifies the gate, its decision context and the grant against Core's
//! `laws/capability-gate-laws.json` (family `capabilityGateLaws`, the reference's
//! D255): the identity a grant is bound to, whether a grant covers an
//! invocation, the registry's named decision, and dispatch.
//!
//! The registry, the contexts and the gate table are built from the FILE's head,
//! as the reference's own read-back does, so a head that no longer declares the
//! registry the vectors were computed against fails here too. Every vector is
//! decided twice: under the file's string labels, and under an integer context
//! (each label's index in `contexts`), the context an embedded host decides on.
//! The family's size is its row in `laws/manifest.json`, and every leg asserts
//! the count of each case it ran.
//!
//! The identity and a grant's coverage are answered in the forms a decision
//! uses, which build nothing (DECISIONS.md D12): the identity written through a
//! writer, and the grant compared with it as it is written. Each is asserted to
//! agree with the built form, the identity's `String` and `Grant::covers`.

mod common;

use std::cell::Cell;
use std::sync::Arc;

use common::*;
use fuaran_core::capability::{
    Capability, CapabilityRegistry, Grant, InvokeError, PolicyDecision, PolicyGate,
};
use fuaran_core::json::{JVal, render_canonical};

const FILE: &str = "laws/capability-gate-laws.json";
const CASES: [&str; 4] = [
    "invocationIdentity",
    "grantCovers",
    "decideNamed",
    "dispatch",
];

fn doc() -> JVal {
    let d = read(FILE);
    assert_eq!(s(&d, "family"), "capabilityGateLaws");
    d
}

fn args_of(input: &JVal) -> Vec<(String, String)> {
    arr(input, "args")
        .iter()
        .map(|a| (s(a, "addr").to_string(), s(a, "value").to_string()))
        .collect()
}

fn grant_of(input: &JVal) -> Option<Grant> {
    input.field("grant").map(|g| match g {
        JVal::Str(identity) => Grant::of_invocation(identity.clone()),
        other => panic!("`grant` is not a string: {other:?}"),
    })
}

fn decision_members(d: PolicyDecision) -> Vec<(String, JVal)> {
    match d {
        PolicyDecision::Allow => vec![("decision".into(), JVal::Str("allow".into()))],
        PolicyDecision::NeedsApproval => {
            vec![("decision".into(), JVal::Str("needsApproval".into()))]
        }
        PolicyDecision::Deny {
            message,
            alternatives,
        } => vec![
            ("decision".into(), JVal::Str("deny".into())),
            ("message".into(), JVal::Str(message.to_string())),
            ("alternatives".into(), shared(&alternatives)),
        ],
    }
}

fn strs(xs: Vec<String>) -> JVal {
    JVal::Arr(xs.into_iter().map(JVal::Str).collect())
}

fn shared(xs: &[Arc<str>]) -> JVal {
    JVal::Arr(xs.iter().map(|x| JVal::Str(x.to_string())).collect())
}

/// A refusal in the file's words; one the family does not name is `unexpected`.
fn refusal_members(e: InvokeError) -> Vec<(String, JVal)> {
    let str = |x: &str| JVal::Str(x.to_string());
    match e {
        InvokeError::NoSuchCapability { id, known } => vec![
            ("error".into(), str("noSuchCapability")),
            ("id".into(), JVal::Str(id)),
            ("known".into(), strs(known)),
        ],
        InvokeError::ArgOutOfSpace { addr, .. } => vec![
            ("error".into(), str("argOutOfSpace")),
            ("addr".into(), JVal::Str(addr)),
        ],
        InvokeError::PolicyRefused {
            policy,
            reason,
            allowed,
        } => vec![
            ("error".into(), str("policyRefused")),
            ("policy".into(), str(&policy)),
            ("reason".into(), str(&reason)),
            ("allowed".into(), shared(&allowed)),
        ],
        InvokeError::ApprovalRequired { policy } => vec![
            ("error".into(), str("approvalRequired")),
            ("policy".into(), str(&policy)),
        ],
        other => vec![
            ("error".into(), str("unexpected")),
            ("detail".into(), JVal::Str(other.describe())),
        ],
    }
}

/// The registry the file's head declares, deciding under contexts of type `X`:
/// its capabilities (each declaration held to its own canonical bytes), and one
/// gate per table row answering its rule's decision for the (context,
/// capability) pair and `allow` for any other. `ctx_of` maps a label to `X`.
fn registry_of<X>(d: &JVal, ctx_of: &dyn Fn(&str) -> X) -> CapabilityRegistry<X>
where
    X: PartialEq + Send + Sync + 'static,
{
    let mut r = CapabilityRegistry::<X>::empty();
    for text in str_items(d, "registry") {
        let c = Capability::decode(&text).unwrap_or_else(|e| panic!("{text}: {e:?}"));
        assert_eq!(c.encode(), text, "a registry declaration's canonical bytes");
        r = r
            .register(c)
            .unwrap_or_else(|e| panic!("the declared registry: {}", e.describe()));
    }
    for gate in arr(d, "gates") {
        let rules: Vec<(X, String, PolicyDecision)> = arr(gate, "rules")
            .iter()
            .map(|rule| {
                (
                    ctx_of(s(rule, "context")),
                    s(rule, "capability").to_string(),
                    decision_of(rule),
                )
            })
            .collect();
        r = r.with_gate(PolicyGate::new(
            s(gate, "policy"),
            move |ctx: &X, c: &Capability, _: &[(String, String)]| {
                rules
                    .iter()
                    .find(|(x, cap, _)| x == ctx && *cap == c.id)
                    .map_or(PolicyDecision::Allow, |(_, _, d)| d.clone())
            },
        ));
    }
    r
}

/// What the port answers for one vector, in the file's members.
fn answer<X>(r: &CapabilityRegistry<X>, ctx_of: &dyn Fn(&str) -> X, v: &JVal) -> JVal {
    let input = get(v, "input");
    let id = s(input, "capability");
    let args = args_of(input);
    let grant = grant_of(input);
    let cap = || {
        r.try_find(id)
            .unwrap_or_else(|| panic!("{id} is not declared in the head"))
    };
    let members: Vec<(String, JVal)> = match s(v, "case") {
        "invocationIdentity" => {
            let mut written = String::new();
            cap()
                .write_invocation_identity(&args, &mut written)
                .expect("a String accepts every write");
            assert_eq!(written, cap().invocation_identity(&args), "{}", s(v, "id"));
            vec![("identity".into(), JVal::Str(written))]
        }
        "grantCovers" => {
            let covers = grant
                .as_ref()
                .is_some_and(|g| g.covers_invocation(cap(), &args));
            let identity = cap().invocation_identity(&args);
            assert_eq!(
                covers,
                grant.as_ref().is_some_and(|g| g.covers(&identity)),
                "{}",
                s(v, "id")
            );
            vec![("covers".into(), JVal::Bool(covers))]
        }
        "decideNamed" => {
            let ctx = ctx_of(s(input, "context"));
            match r.decide_named(&ctx, grant.as_ref(), id, &args) {
                Ok((policy, d)) => {
                    let mut m = vec![
                        ("verdict".into(), JVal::Str("decided".into())),
                        ("policy".into(), JVal::Str(policy.to_string())),
                    ];
                    m.extend(decision_members(d));
                    m
                }
                Err(e) => {
                    let mut m = vec![("verdict".into(), JVal::Str("refused".into()))];
                    m.extend(refusal_members(e));
                    m
                }
            }
        }
        "dispatch" => {
            let ctx = ctx_of(s(input, "context"));
            let ran = Cell::new(0u32);
            let out = r.dispatch(&ctx, grant.as_ref(), id, &args, |_| {
                ran.set(ran.get() + 1);
                Ok(())
            });
            match (out, ran.get()) {
                (Ok(()), 1) => vec![("outcome".into(), JVal::Str("performed".into()))],
                (Err(e), 0) => {
                    let mut m = vec![("outcome".into(), JVal::Str("refused".into()))];
                    m.extend(refusal_members(e));
                    m
                }
                (out, n) => vec![
                    ("outcome".into(), JVal::Str("unexpected".into())),
                    (
                        "detail".into(),
                        JVal::Str(format!("{out:?} after {n} run(s)")),
                    ),
                ],
            }
        }
        other => panic!("{}: unknown case {other}", s(v, "id")),
    };
    JVal::Obj(members)
}

/// Decide every vector under contexts of type `X`; the count of each case run.
/// The family's line is printed once, from the run under the file's own labels.
fn certify<X>(ctx_of: &dyn Fn(&str) -> X) -> Vec<usize>
where
    X: PartialEq + Send + Sync + 'static,
{
    let d = doc();
    let r = registry_of(&d, ctx_of);
    let mut ran = vec![0usize; CASES.len()];
    for v in vectors(&d) {
        let id = s(v, "id");
        assert_eq!(
            render_canonical(&answer(&r, ctx_of, v)),
            render_canonical(get(v, "expected")),
            "{id}"
        );
        let at = CASES
            .iter()
            .position(|c| *c == s(v, "case"))
            .unwrap_or_else(|| panic!("{id}: a case outside the family"));
        ran[at] += 1;
    }
    ran
}

#[test]
fn the_family_is_the_four_cases_its_manifest_counts() {
    let d = doc();
    assert_eq!(vectors(&d).len(), declared("laws", "capabilityGateLaws"));
    assert_eq!(vectors(&d).len(), 41);
    let mut order: Vec<&str> = Vec::new();
    for v in vectors(&d) {
        if !order.contains(&s(v, "case")) {
            order.push(s(v, "case"));
        }
    }
    assert_eq!(order, CASES, "the four cases, in order");
    assert_eq!(str_items(&d, "contexts"), ["trusted", "untrusted"]);
}

#[test]
fn every_vector_under_the_files_context_labels() {
    let ran = certify(&|label: &str| label.to_string());
    answered("laws", "capabilityGateLaws", ran.iter().sum());
    assert_eq!(ran, [9, 5, 20, 7]);
}

#[test]
fn every_vector_under_an_integer_context() {
    let contexts = str_items(&doc(), "contexts");
    let ran = certify(&|label: &str| {
        contexts
            .iter()
            .position(|c| c == label)
            .unwrap_or_else(|| panic!("{label} is not a declared context")) as u8
    });
    assert_eq!(ran, [9, 5, 20, 7]);
}

/// The identity is compared as bytes with the vector's, and it is never the
/// hashed capture key: a grant spelled as the key covers nothing.
#[test]
fn the_identity_is_bytes_and_never_the_capture_key() {
    let d = doc();
    let r = registry_of(&d, &|label: &str| label.to_string());
    let mut ran = 0;
    for v in vectors(&d) {
        if s(v, "case") != "invocationIdentity" {
            continue;
        }
        let input = get(v, "input");
        let c = r.try_find(s(input, "capability")).unwrap();
        let args = args_of(input);
        let identity = c.invocation_identity(&args);
        assert_eq!(
            identity.as_bytes(),
            s(get(v, "expected"), "identity").as_bytes(),
            "{}",
            s(v, "id")
        );
        assert_eq!(c.grant(&args), Grant::of_invocation(identity.clone()));
        let key = c.invocation_key(&args);
        assert_ne!(key, identity, "{}", s(v, "id"));
        assert!(
            !Grant::of_invocation(key).covers(&identity),
            "{}",
            s(v, "id")
        );
        ran += 1;
    }
    assert_eq!(ran, 9);
}

/// `under` fixes a context and loosens nothing: the view decides every
/// `decideNamed` and `dispatch` vector of that context exactly as the registry
/// does under it.
#[test]
fn a_fixed_context_view_decides_as_the_registry_does_under_it() {
    let d = doc();
    let r = registry_of(&d, &|label: &str| label.to_string());
    let mut ran = 0;
    for v in vectors(&d) {
        if !matches!(s(v, "case"), "decideNamed" | "dispatch") {
            continue;
        }
        let input = get(v, "input");
        let ctx = s(input, "context").to_string();
        let view = r.under(ctx.clone());
        let (id, args, grant) = (s(input, "capability"), args_of(input), grant_of(input));
        assert_eq!(view.gates(), r.gates());
        assert_eq!(
            view.decide_named(&(), grant.as_ref(), id, &args),
            r.decide_named(&ctx, grant.as_ref(), id, &args),
            "{}",
            s(v, "id")
        );
        assert_eq!(
            view.dispatch(&(), grant.as_ref(), id, &args, |_| Ok(())),
            r.dispatch(&ctx, grant.as_ref(), id, &args, |_| Ok(())),
            "{}",
            s(v, "id")
        );
        ran += 1;
    }
    assert_eq!(ran, 27);
}
