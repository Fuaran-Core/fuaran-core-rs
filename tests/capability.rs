//! Certifies the capability contract against Core's `laws/capability-laws.json`
//! (family `capabilityLaws`): argument validation, the invocation key and its
//! determinism tag, the declaration codec's round-trip with the declared result,
//! the declaration reader's refusals, id-ordered registry enumeration, and the
//! pipeline type check of a call's declared output against that result. And,
//! through the same four of those cases, the document value space against
//! `laws/capability-document-laws.json` (family `capabilityDocumentLaws`, the
//! reference's D260).
//!
//! Each family's size is its row in `laws/manifest.json` (for `capabilityLaws`
//! also the file's own `iterations` times the twelve vectors each iteration
//! draws); every leg asserts the exact count of the case it ran.

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

// ─── laws/capability-document-laws.json — the document value space ──────────

/// The document family (`capabilityDocumentLaws`, the reference's D260): the
/// cases of `capability-laws.json`, read exactly as that file describes them,
/// over one capability whose hole and result are documents.
const DOCUMENT_FILE: &str = "laws/capability-document-laws.json";

fn document_doc() -> JVal {
    let d = read(DOCUMENT_FILE);
    assert_eq!(s(&d, "family"), "capabilityDocumentLaws");
    d
}

#[test]
fn the_document_family_is_the_four_cases_its_manifest_counts() {
    let d = document_doc();
    assert_eq!(
        vectors(&d).len(),
        declared("laws", "capabilityDocumentLaws")
    );
    assert_eq!(vectors(&d).len(), 18);
    let mut order: Vec<&str> = Vec::new();
    for v in vectors(&d) {
        if !order.contains(&s(v, "case")) {
            order.push(s(v, "case"));
        }
    }
    assert_eq!(
        order,
        [
            "validateArgs",
            "invocationKey",
            "declarationRoundTrip",
            "pipelineTypeCheck"
        ],
        "the four cases, in order"
    );
}

#[test]
fn document_validate_args() {
    let d = document_doc();
    let mut ran = 0;
    for v in of_case(&d, "validateArgs") {
        let id = s(v, "id");
        let c = cap_of(id, s(get(v, "input"), "capability"));
        let got = verdict(c.validate_args(&args_of(v)));
        assert_eq!(got, render_canonical(get(v, "expected")), "{id}");
        ran += 1;
    }
    assert_eq!(ran, 9);
}

#[test]
fn document_invocation_key() {
    let d = document_doc();
    let mut ran = 0;
    for v in of_case(&d, "invocationKey") {
        let id = s(v, "id");
        let c = cap_of(id, s(get(v, "input"), "capability"));
        let args = args_of(v);
        let e = get(v, "expected");
        assert_eq!(
            c.invocation_key(&args),
            s(e, "key"),
            "{id}: the invocation key"
        );
        assert_eq!(c.determinism_tag(), s(e, "determinismTag"), "{id}: the tag");
        assert!(
            e.field("capturedValue").is_none(),
            "{id}: a document key vector carries no capturedValue"
        );
        ran += 1;
    }
    assert_eq!(ran, 2);
}

#[test]
fn document_declaration_round_trip() {
    let d = document_doc();
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
    assert_eq!(ran, 2);
}

#[test]
fn document_pipeline_type_check() {
    let d = document_doc();
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
    assert_eq!(ran, 5);
}

// ─── The document space, beyond the vectors ──────────────────────────────────

/// What the family's vectors do not reach, held to the reference's rules
/// (DECISIONS.md D13): a number no finite double spells, a `null` below the
/// root, an unpaired surrogate and a bare non-finite token are each no
/// document; the refusal is `ArgOutOfSpace` carrying the space, in words.
#[test]
fn a_document_holds_no_null_no_non_finite_number_and_no_ill_formed_string() {
    let d = document_doc();
    let text = s(get(&vectors(&d)[0], "input"), "capability");
    let c = cap_of("document-space", text);
    for (doc, member) in [
        (r#"{"a":1e400}"#, false),
        (r#"[-1e400]"#, false),
        (r#"{"a":NaN}"#, false),
        (r#"[Infinity]"#, false),
        (r#"{"a":[1,{"b":null}]}"#, false),
        (r#""\ud800""#, false),
        (r#"{"a":"\ud83d\ude00"}"#, true),
        (r#"{"a":1.7976931348623157e308}"#, true),
        (r#"-0"#, true),
        (r#"true"#, true),
        (r#"{}"#, true),
    ] {
        let got = c.validate_args(&[("d0".to_string(), doc.to_string())]);
        if member {
            assert_eq!(got, Ok(()), "{doc} is a document");
        } else {
            match got {
                Err(InvokeError::ArgOutOfSpace { addr, space, got }) => {
                    assert_eq!((addr.as_str(), got.as_str()), ("d0", doc));
                    assert_eq!(
                        space.describe(),
                        "a document of type 'Order': well-formed JSON"
                    );
                }
                other => panic!("{doc}: {other:?}"),
            }
        }
    }
}

/// One document, one spelling: the canonical text sorts members by UTF-16 code
/// unit, writes the canonical number form and the canonical escape, and
/// collapses a negative zero; a string that is no document has none.
#[test]
fn a_document_has_one_canonical_text() {
    use fuaran_core::capability::document_of;
    assert_eq!(
        document_of(
            r#"{ "b" : [ true, "x\ny", 2.50 ],
  "a" : 1 }"#
        )
        .as_deref(),
        Some(r#"{"a":1,"b":[true,"x\u000ay",2.5]}"#)
    );
    assert_eq!(
        document_of("{\"\u{e000}\":1,\"\u{1f600}\":2}").as_deref(),
        Some("{\"\u{1f600}\":2,\"\u{e000}\":1}"),
        "members sorted by UTF-16 code unit, not by scalar value"
    );
    assert_eq!(document_of("[-0.0,1E2]").as_deref(), Some("[0,100]"));
    assert_eq!(document_of("{\"a\":1,\"a\":1}"), None);
    assert_eq!(document_of("null"), None);
    assert_eq!(document_of(""), None);
}

/// The space's codec and its relation: the type name round-trips and is read
/// as a string; the documents are a family of their own.
#[test]
fn the_document_space_codec_and_relation() {
    use fuaran_core::capability::ValueSpace::{self, *};
    let order = || Document(Some("Order".to_string()));
    let receipt = || Document(Some("Receipt".to_string()));
    for (required, available, admits) in [
        (Document(None), order(), true),
        (Document(None), Document(None), true),
        (order(), order(), true),
        (order(), receipt(), false),
        (order(), Document(None), false),
        (AnyString, order(), false),
        (SlotTree(None), Document(None), false),
        (Document(None), SlotTree(None), false),
        (Document(None), AnyString, false),
    ] {
        assert_eq!(
            required.subsumes(&available),
            admits,
            "{required:?} admits {available:?}"
        );
    }
    let space_of_text = |text: &str| -> Result<ValueSpace, fuaran_core::decode::DecodeError> {
        let v = fuaran_core::decode::parse(text, &fuaran_core::json::JsonLimits::STRICT)?;
        fuaran_core::capability::space_of(&fuaran_core::decode::Cursor::root(&v))
    };
    for space in [Document(None), order()] {
        let text = render_canonical(&fuaran_core::capability::space_to_json(&space));
        assert_eq!(space_of_text(&text), Ok(space), "{text}");
    }
    assert_eq!(
        render_canonical(&fuaran_core::capability::space_to_json(&order())),
        r#"{"$type":"document","documentType":"Order"}"#
    );
    let e = space_of_text(r#"{"$type":"document","documentType":1}"#).unwrap_err();
    assert_eq!(e.code, fuaran_core::decode::DecodeCode::WrongKind);
    assert_eq!(Document(None).describe(), "a document: well-formed JSON");
    assert_eq!(Document(None).well_formed(), Ok(()));
    assert!(!Document(None).is_count());
}

/// A float argument that no finite double spells is out of a number space, as
/// the reference refuses it (D260.7): the token, the overflow and the sentinel.
#[test]
fn a_number_space_refuses_a_non_finite_argument() {
    let c = Capability::decode(r#"{"$type":"capability","determinism":"deterministic","id":"f","placement":{"$type":"server"},"result":{"$type":"nothing"},"signature":{"effect":{"determinism":"deterministic","host":"pure"},"holes":[{"addr":"x","kind":"value","name":"x","required":true,"space":{"$type":"floatRange","max":1.7976931348623157e308,"min":-1.7976931348623157e308}}],"name":"f"}}"#)
        .expect("a declaration");
    for x in ["NaN", "Infinity", "-Infinity", "1e400", "-1e400"] {
        assert!(
            matches!(
                c.validate_args(&[("x".to_string(), x.to_string())]),
                Err(InvokeError::ArgOutOfSpace { .. })
            ),
            "{x}"
        );
    }
    assert_eq!(
        c.validate_args(&[("x".to_string(), "1e308".to_string())]),
        Ok(())
    );
}
