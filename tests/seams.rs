//! The seam-refusal family: every seam document read by the declaration-driven
//! codec, answering the reference's re-encoding or its refusal on code and path.

mod common;
use common::*;

use fuaran_core::decode::{self, DecodeError};
use fuaran_core::json::{JVal, JsonLimits, PathSeg};
use fuaran_core::seam::{self, Reading, Slot};

fn path_json(e: &DecodeError) -> Vec<JVal> {
    e.path
        .iter()
        .map(|s| match s {
            PathSeg::Key(k) => JVal::Str(k.clone()),
            PathSeg::Index(i) => JVal::Num(*i as f64),
        })
        .collect()
}

#[test]
fn seam_refusals() {
    let doc = read("refusals/seam-refusals.json");
    let all = vectors(&doc);
    assert_eq!(all.len(), declared("refusals", "seamRefusals"));

    let mut ran = 0usize;
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
        match v.field("canonical") {
            Some(JVal::Str(canonical)) => {
                let out = got.unwrap_or_else(|e| panic!("{reader}/{name}: refused {e:?}"));
                assert_eq!(
                    &fuaran_core::json::render_canonical(&out),
                    canonical,
                    "{reader}/{name}"
                );
            }
            _ => {
                let e = match got {
                    Err(e) => e,
                    Ok(out) => panic!(
                        "{reader}/{name}: read {} where {} was expected",
                        fuaran_core::json::render_canonical(&out),
                        s(v, "code")
                    ),
                };
                assert_eq!(e.code.as_str(), s(v, "code"), "{reader}/{name}: {e:?}");
                assert_eq!(&path_json(&e), arr(v, "path"), "{reader}/{name}: {e:?}");
            }
        }
        ran += 1;
    }
    assert_eq!(ran, declared("refusals", "seamRefusals"));
}

#[test]
fn every_reader_the_family_names_is_declared() {
    let doc = read("refusals/seam-refusals.json");
    for v in vectors(&doc) {
        assert!(seam::reader(s(v, "reader")).is_some(), "{}", s(v, "reader"));
    }
}
