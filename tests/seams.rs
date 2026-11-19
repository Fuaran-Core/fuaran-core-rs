//! The seam-refusal family: every seam document read by the declaration-driven
//! codec, answering the reference's re-encoding or its refusal on code and path.

mod common;
use common::*;

use fuaran_core::decode;
use fuaran_core::json::{JVal, JsonLimits};
use fuaran_core::seam::{self, Reading, Slot};

#[test]
fn seam_refusals() {
    let doc = read("refusals/seam-refusals.json");
    let all = vectors(&doc);
    assert_eq!(all.len(), declared("refusals", "seamRefusals"));

    // Every vector is answered before the leg judges, so a red names each vector
    // answered differently, not only the first.
    let mut ran = 0usize;
    let mut wrong = Vec::new();
    for v in all {
        let name = s(v, "name");
        let reader = s(v, "reader");
        let (root, strict) =
            seam::reader(reader).unwrap_or_else(|| panic!("no reader '{reader}' ({name})"));
        let reading = Reading {
            strict,
            param: Slot::Str,
        };
        let parsed = decode::parse(s(v, "document"), &JsonLimits::STRICT)
            .unwrap_or_else(|e| panic!("{reader}/{name}: document unreadable: {e:?}"));
        let got = seam::read(&root, &parsed, &reading);
        let differs = match (v.field("canonical"), got) {
            (Some(JVal::Str(canonical)), Ok(out)) => {
                let out = fuaran_core::json::render_canonical(&out);
                (&out != canonical).then(|| format!("read {out} where {canonical} was expected"))
            }
            (Some(JVal::Str(canonical)), Err(e)) => {
                Some(format!("refused {e:?} where {canonical} was expected"))
            }
            (_, Ok(out)) => Some(format!(
                "read {} where {} was expected",
                fuaran_core::json::render_canonical(&out),
                s(v, "code")
            )),
            (_, Err(e)) => (e.code.as_str() != s(v, "code") || path_json(&e) != arr(v, "path"))
                .then(|| {
                    format!(
                        "refused {e:?} where {} at {:?} was expected",
                        s(v, "code"),
                        arr(v, "path")
                    )
                }),
        };
        if let Some(why) = differs {
            wrong.push(format!("{reader}/{name}: {why}"));
        }
        ran += 1;
    }
    assert!(
        wrong.is_empty(),
        "{} of {ran} vectors answered differently:\n{}",
        wrong.len(),
        wrong.join("\n")
    );
    answered("refusals", "seamRefusals", ran);
}

#[test]
fn every_reader_the_family_names_is_declared() {
    let doc = read("refusals/seam-refusals.json");
    for v in vectors(&doc) {
        assert!(seam::reader(s(v, "reader")).is_some(), "{}", s(v, "reader"));
    }
}
