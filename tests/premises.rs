//! Certifies the spine's runtime premises against the probes the reference emits
//! (`conformance/premises/`): each family is the probe a `host-premise` row of the
//! reference's claims ladder names, and reproducing every vector discharges that
//! premise on this crate's own runtime (the reference's docs/host-premises.md).
//!
//! Each leg asserts it ran exactly the count the family's manifest declares.

mod common;

use common::*;
use fuaran_core::decode::{self, Value};
use fuaran_core::json::{self, JVal, JsonLimits, JsonRefusal, render_canonical};
use fuaran_core::number::format_finite_double;
use fuaran_core::sha256;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).expect("hex"))
        .collect()
}

/// A refusal's class as the probes spell it: the malformed-text kind, or
/// `LimitExceeded/<bound>` for a well-formed document past a bound.
fn refusal_class(r: &JsonRefusal) -> String {
    match r {
        JsonRefusal::Malformed(e) => e.kind.as_str().to_string(),
        JsonRefusal::LimitExceeded(b) => format!("LimitExceeded/{}", b.limit.as_str()),
    }
}

/// The premise families' manifest is `premises/manifest.json`.
fn family(file: &str, id: &str) -> (JVal, usize) {
    let doc = read(&format!("premises/{file}"));
    let want = declared("premises", id);
    assert_eq!(vectors(&doc).len(), want, "{file}: the manifest's count");
    (doc, want)
}

// ─── premises/string-model.json ──────────────────────────────────────────────

/// The crate's strings are UTF-8 (`str`), so a sequence of UTF-16 code units
/// reaches it only through this mapping: a well-formed sequence becomes its
/// UTF-8 bytes, and the first unpaired surrogate refuses at its index.
fn utf8_of_units(units: &[u16]) -> Result<Vec<u8>, (usize, u16)> {
    let mut out = String::new();
    let mut at = 0;
    for c in char::decode_utf16(units.iter().copied()) {
        match c {
            Ok(c) => {
                out.push(c);
                at += c.len_utf16();
            }
            Err(e) => return Err((at, e.unpaired_surrogate())),
        }
    }
    Ok(out.into_bytes())
}

#[test]
fn premise_string_model() {
    let (doc, want) = family("string-model.json", "premiseStringModel");
    let mut ran = 0;
    for v in vectors(&doc) {
        let (id, expected) = (s(v, "id"), get(v, "expected"));
        if id.starts_with("encode/") {
            let units: Vec<u16> = arr(v, "units")
                .iter()
                .map(|u| match u {
                    JVal::Num(x) => *x as u16,
                    other => panic!("{id}: a unit is {other:?}"),
                })
                .collect();
            match utf8_of_units(&units) {
                Ok(bytes) => assert_eq!(hex(&bytes), s(expected, "utf8"), "{id}"),
                Err((index, unit)) => {
                    let refused = get(expected, "refused");
                    assert_eq!(index, n(refused, "index") as usize, "{id}: index");
                    assert_eq!(unit, n(refused, "unit") as u16, "{id}: unit");
                }
            }
        } else {
            assert!(id.starts_with("parse/"), "{id}: an unknown probe");
            match json::parse(s(v, "text")) {
                Err(r) => assert_eq!(refusal_class(&r), s(expected, "refused"), "{id}"),
                Ok(JVal::Str(text)) => {
                    assert_eq!(hex(text.as_bytes()), s(expected, "utf8"), "{id}")
                }
                Ok(JVal::Obj(members)) => {
                    assert_eq!(members.len(), 1, "{id}: one member");
                    let key = get(expected, "key");
                    assert_eq!(hex(members[0].0.as_bytes()), s(key, "utf8"), "{id}");
                }
                Ok(other) => panic!("{id}: read {other:?}"),
            }
        }
        ran += 1;
    }
    assert_eq!(ran, want);
}

// ─── premises/float-digits.json ──────────────────────────────────────────────

#[test]
fn premise_float_digits() {
    let (doc, want) = family("float-digits.json", "premiseFloatDigits");
    let mut ran = 0;
    for v in vectors(&doc) {
        let id = s(v, "id");
        let bits = u64::from_str_radix(s(v, "bits"), 16).expect("bits");
        let canonical = s(get(v, "expected"), "canonical");
        let f = f64::from_bits(bits);
        assert_eq!(format_finite_double(f), canonical, "{id}: written");
        // Read back: the same double, the negative zero reading as zero.
        let back = match json::parse(canonical) {
            Ok(JVal::Num(x)) => x,
            other => panic!("{id}: {canonical} read back as {other:?}"),
        };
        let wanted = if f == 0.0 { 0u64 } else { bits };
        assert_eq!(back.to_bits(), wanted, "{id}: read back");
        ran += 1;
    }
    assert_eq!(ran, want);
}

// ─── premises/int53.json ─────────────────────────────────────────────────────

#[test]
fn premise_int53() {
    let (doc, want) = family("int53.json", "premiseInt53");
    let mut ran = 0;
    for v in vectors(&doc) {
        let (id, expected) = (s(v, "id"), get(v, "expected"));
        match decode::read_value(s(v, "token"), &JsonLimits::STRICT) {
            Ok(Value::Int(i)) => assert_eq!(i as f64, n(expected, "int"), "{id}"),
            Ok(Value::Float(f)) => assert_eq!(
                format!("{:016x}", f.to_bits()),
                s(expected, "float"),
                "{id}"
            ),
            Ok(other) => panic!("{id}: read {other:?}"),
            Err(r) => assert_eq!(refusal_class(&r), s(expected, "refused"), "{id}"),
        }
        ran += 1;
    }
    assert_eq!(ran, want);
}

// ─── premises/sha256.json ────────────────────────────────────────────────────

#[test]
fn premise_sha256() {
    let (doc, want) = family("sha256.json", "premiseSha256");
    let mut ran = 0;
    for v in vectors(&doc) {
        let (id, input) = (s(v, "id"), get(v, "input"));
        let message = unhex(s(input, "hex")).repeat(n(input, "repeat") as usize);
        assert_eq!(
            hex(&sha256::sha256(&message)),
            s(get(v, "expected"), "sha256"),
            "{id}"
        );
        ran += 1;
    }
    assert_eq!(ran, want);
}

// ─── premises/recursion-depth.json ───────────────────────────────────────────

/// The stack the depth probes run on, stated rather than inherited from the test
/// harness, and small: the reader and the canonical writer keep their nesting on
/// the heap, so the native stack they use is the same at every depth, and the
/// depths are read, refused and written back on a stack a microcontroller has.
/// Measured on x86_64 (DECISIONS.md D10): both run in 8 KiB, debug and release.
/// The leg runs debug under `cargo test` and release under `run.ps1`'s release leg.
const PROBE_STACK_BYTES: usize = 64 * 1024;

#[test]
fn premise_recursion_depth() {
    let (doc, want) = family("recursion-depth.json", "premiseRecursionDepth");
    let (ran, accepted) = std::thread::Builder::new()
        .stack_size(PROBE_STACK_BYTES)
        .spawn(move || {
            let mut ran = 0;
            let mut accepted = Vec::new();
            for v in vectors(&doc) {
                let (id, input, expected) = (s(v, "id"), get(v, "input"), get(v, "expected"));
                let depth = n(input, "depth") as usize;
                let text = format!(
                    "{}{}{}",
                    s(input, "open").repeat(depth),
                    s(input, "inner"),
                    s(input, "close").repeat(depth)
                );
                match json::parse(&text) {
                    Ok(value) => {
                        let rendered = render_canonical(&value);
                        assert_eq!(rendered, text, "{id}: the canonical rendering");
                        assert_eq!(
                            sha256::sha256_hex(&rendered),
                            s(expected, "accepted"),
                            "{id}"
                        );
                        accepted.push(value);
                    }
                    Err(r) => assert_eq!(refusal_class(&r), s(expected, "refused"), "{id}"),
                }
                ran += 1;
            }
            // Handed back rather than dropped here: a value's drop glue is the
            // compiler's, recursive in the value's depth, and not the reader's
            // (DECISIONS.md D10 measures it). The premise is the reader's.
            (ran, accepted)
        })
        .expect("the probe thread starts")
        .join()
        .expect("the depth probes ran without exhausting the stack");
    assert_eq!(ran, want);
    drop(accepted);
}
