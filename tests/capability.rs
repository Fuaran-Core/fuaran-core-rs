//! Certifies the capability contract against Core's `laws/capability-laws.json`
//! (family `capabilityLaws`): argument validation, the invocation key and its
//! determinism tag, the declaration codec's round-trip with the declared result,
//! the declaration reader's refusals, id-ordered registry enumeration, and the
//! pipeline type check of a call's declared output against that result.
//!
//! The family's size is its row in `laws/manifest.json`, and the file's own
//! `iterations` times the twelve vectors each iteration draws; every leg asserts
//! the exact count of the case it ran.

mod common;

use common::*;
use fuaran_core::capability::{
    Capability, CapabilityPipeline, CapabilityRegistry, InvokeError, PipelineError,
};
use fuaran_core::json::{JVal, render_canonical};

const FILE: &str = "laws/capability-laws.json";

fn doc() -> JVal {
    let d = read(FILE);
    assert_eq!(s(&d, "family"), "capabilityLaws");
    d
}

fn iterations(d: &JVal) -> usize {
    n(d, "iterations") as usize
}

fn of_case<'a>(d: &'a JVal, case: &str) -> Vec<&'a JVal> {
    vectors(d).iter().filter(|v| s(v, "case") == case).collect()
}

/// A declaration string, decoded, and held to its own bytes: the corpus carries
/// canonical declarations, so decode-then-encode must return them.
fn cap_of(id: &str, text: &str) -> Capability {
    let c = Capability::decode(text).unwrap_or_else(|e| panic!("{id}: declaration refused: {e:?}"));
    assert_eq!(c.encode(), text, "{id}: the declaration's canonical bytes");
    c
}

fn args_of(v: &JVal) -> Vec<(String, String)> {
    arr(get(v, "input"), "args")
        .iter()
        .map(|a| (s(a, "addr").to_string(), s(a, "value").to_string()))
        .collect()
}

/// The verdict in the file's words: accept, or reject naming the two error
/// classes the law distinguishes (anything else is `unexpected`).
fn verdict(r: Result<(), InvokeError>) -> String {
    let members: Vec<(String, JVal)> = match r {
        Ok(()) => vec![("verdict".into(), JVal::Str("accept".into()))],
        Err(InvokeError::ArgOutOfSpace { addr, .. }) => vec![
            ("verdict".into(), JVal::Str("reject".into())),
            ("error".into(), JVal::Str("argOutOfSpace".into())),
            ("addr".into(), JVal::Str(addr)),
        ],
        Err(InvokeError::UnknownArg { addr, .. }) => vec![
            ("verdict".into(), JVal::Str("reject".into())),
            ("error".into(), JVal::Str("unknownArg".into())),
            ("addr".into(), JVal::Str(addr)),
        ],
        Err(_) => vec![
            ("verdict".into(), JVal::Str("reject".into())),
            ("error".into(), JVal::Str("unexpected".into())),
        ],
    };
    render_canonical(&JVal::Obj(members))
}

#[test]
fn the_family_is_twelve_vectors_per_iteration() {
    let d = doc();
    assert_eq!(vectors(&d).len(), 12 * iterations(&d));
    assert_eq!(vectors(&d).len(), declared("laws", "capabilityLaws"));
}

#[test]
fn validate_args() {
    let d = doc();
    let mut ran = 0;
    for v in of_case(&d, "validateArgs") {
        let id = s(v, "id");
        let c = cap_of(id, s(get(v, "input"), "capability"));
        let got = verdict(c.validate_args(&args_of(v)));
        assert_eq!(got, render_canonical(get(v, "expected")), "{id}");
        ran += 1;
    }
    assert_eq!(ran, 5 * iterations(&d));
    assert_eq!(ran, 60);
}

#[test]
fn invocation_key() {
    let d = doc();
    let mut ran = 0;
    for v in of_case(&d, "invocationKey") {
        let id = s(v, "id");
        let c = cap_of(id, s(get(v, "input"), "capability"));
        let args = args_of(v);
        let e = get(v, "expected");
        let key = c.invocation_key(&args);
        assert_eq!(key, s(e, "key"), "{id}: the invocation key");
        assert_eq!(c.determinism_tag(), s(e, "determinismTag"), "{id}: the tag");
        // Every argument is keyed by its canonical spelling: a leading zero is the
        // same integer, and so the same key.
        let respelled: Vec<(String, String)> = args
            .iter()
            .map(|(a, x)| (a.clone(), format!("0{x}")))
            .collect();
        assert_eq!(
            c.invocation_key(&respelled),
            key,
            "{id}: one value, one key"
        );
        // `capturedValue` is the sample's own draw, carried through rather than
        // computed from the inputs (the reference's own oracle treats it so);
        // what is checkable of it is that it is a draw of `intBelow 1000`.
        let captured = n(e, "capturedValue");
        assert!(
            captured.fract() == 0.0 && (0.0..1000.0).contains(&captured),
            "{id}: capturedValue {captured} is not a draw in [0, 1000)"
        );
        ran += 1;
    }
    assert_eq!(ran, iterations(&d));
    assert_eq!(ran, 12);
}

#[test]
fn declaration_round_trip() {
    let d = doc();
    let mut ran = 0;
    for v in of_case(&d, "declarationRoundTrip") {
        let id = s(v, "id");
        let input = s(get(v, "input"), "declaration");
        let c = Capability::decode(input).unwrap_or_else(|e| panic!("{id}: {e:?}"));
        assert_eq!(c.encode(), s(get(v, "expected"), "declaration"), "{id}");
        assert_eq!(
            c.try_encode().as_deref(),
            Ok(input),
            "{id}: the guarded encode"
        );
        ran += 1;
    }
    assert_eq!(ran, iterations(&d));
    assert_eq!(ran, 12);
}

#[test]
fn registry_enumerate() {
    let d = doc();
    let mut ran = 0;
    for v in of_case(&d, "registryEnumerate") {
        let id = s(v, "id");
        let mut r = CapabilityRegistry::new();
        let decls = str_items(get(v, "input"), "declarations");
        for text in &decls {
            r = r
                .register(cap_of(id, text))
                .unwrap_or_else(|e| panic!("{id}: {}", e.describe()));
        }
        let ids: Vec<String> = r.enumerate().iter().map(|c| c.id.clone()).collect();
        assert_eq!(ids, str_items(get(v, "expected"), "ids"), "{id}");
        // The order is the id order, not the insertion order.
        let mut reversed = CapabilityRegistry::new();
        for text in decls.iter().rev() {
            reversed = reversed.register(cap_of(id, text)).unwrap();
        }
        assert_eq!(reversed, r, "{id}: insertion order does not matter");
        ran += 1;
    }
    assert_eq!(ran, iterations(&d));
    assert_eq!(ran, 12);
}

/// A refused declaration in the file's words: its decode code and the path to
/// the fault.
fn refusal(e: &fuaran_core::decode::DecodeError) -> String {
    let path = path_json(e);
    render_canonical(&JVal::Obj(vec![
        ("verdict".into(), JVal::Str("reject".into())),
        ("code".into(), JVal::Str(e.code.as_str().into())),
        ("path".into(), JVal::Arr(path)),
    ]))
}

#[test]
fn declaration_refused() {
    let d = doc();
    let mut ran = 0;
    for v in of_case(&d, "declarationRefused") {
        let id = s(v, "id");
        let e = match Capability::decode(s(get(v, "input"), "declaration")) {
            Err(e) => e,
            Ok(c) => panic!("{id}: admitted {}", c.encode()),
        };
        assert_eq!(
            refusal(&e),
            render_canonical(get(v, "expected")),
            "{id}: {e:?}"
        );
        ran += 1;
    }
    assert_eq!(ran, 2 * iterations(&d));
    assert_eq!(ran, 24);
}

/// A type check's verdict in the file's words: accept, or the one refusal the
/// family names (anything else is `unexpected`, with its detail).
fn typed(r: Result<(), PipelineError>) -> String {
    let members: Vec<(String, JVal)> = match r {
        Ok(()) => vec![("verdict".into(), JVal::Str("accept".into()))],
        Err(PipelineError::PipelineOutputContradicts {
            node, capability, ..
        }) => vec![
            ("verdict".into(), JVal::Str("reject".into())),
            (
                "error".into(),
                JVal::Str("pipelineOutputContradicts".into()),
            ),
            ("node".into(), JVal::Str(node)),
            ("capability".into(), JVal::Str(capability)),
        ],
        Err(other) => vec![
            ("verdict".into(), JVal::Str("reject".into())),
            ("error".into(), JVal::Str("unexpected".into())),
            ("detail".into(), JVal::Str(format!("{other:?}"))),
        ],
    };
    render_canonical(&JVal::Obj(members))
}

#[test]
fn pipeline_type_check() {
    let d = doc();
    let mut ran = 0;
    for v in of_case(&d, "pipelineTypeCheck") {
        let id = s(v, "id");
        let input = get(v, "input");
        let r = CapabilityRegistry::new()
            .register(cap_of(id, s(input, "capability")))
            .unwrap_or_else(|e| panic!("{id}: {}", e.describe()));
        let p = CapabilityPipeline::decode(s(input, "pipeline"))
            .unwrap_or_else(|e| panic!("{id}: the pipeline does not decode: {e:?}"));
        assert_eq!(
            typed(p.type_check(&r)),
            render_canonical(get(v, "expected")),
            "{id}"
        );
        ran += 1;
    }
    assert_eq!(ran, 2 * iterations(&d));
    assert_eq!(ran, 24);
}
