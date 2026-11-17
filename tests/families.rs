//! Every family the reference emits, by name: each family a manifest under
//! Core's `conformance/` declares at the pinned commit is either answered by a
//! leg of this crate or named here as not answered, with the reason and the
//! decision that records it (DECISIONS.md D8, D13, D14, D17).
//!
//! The reference's manifests say a host whose adoption reads `proposed` reports
//! the family BY NAME rather than skipping it. This leg is where the crate does:
//! a family a new pin adds fails here until it is either answered or named, and a
//! family named here that the pin no longer declares fails too, so neither list
//! can go stale silently. What the reference records as the crate's adoption is
//! the reference's to flip; this leg reads the ids, not the records.

mod common;

use std::collections::BTreeSet;
use std::fs;

use common::*;
use fuaran_core::json;

/// The families a leg of this crate answers: the manifest's directory, the
/// family's id, and the leg (`tests/<leg>.rs`) that asserts its count.
const ANSWERED: &[(&str, &str, &str)] = &[
    ("apply", "skeletonApply", "conformance"),
    ("chain", "skeletonChain", "conformance"),
    ("dag", "skeletonDag", "conformance"),
    ("decode", "decodeRejects", "conformance"),
    ("escape", "stringEscape", "conformance"),
    ("diff", "skeletonDiff", "structural"),
    ("merge", "skeletonMerge", "structural"),
    ("laws", "decimal", "conformance"),
    ("laws", "fieldLaws", "columnar"),
    ("laws", "propagationLaws", "propagation_forms"),
    ("laws", "canonicalFormLaws", "propagation_forms"),
    ("laws", "capabilityLaws", "capability"),
    ("laws", "capabilityDocumentLaws", "capability"),
    ("laws", "capabilityGateLaws", "capability_gate"),
    ("laws", "queryGrantLaws", "query_grant"),
    ("laws", "queryDeclarationLaws", "query_declaration"),
    ("refusals", "codecRefusals", "conformance"),
    ("refusals", "seamRefusals", "seams"),
    ("premises", "premiseStringModel", "premises"),
    ("premises", "premiseFloatDigits", "premises"),
    ("premises", "premiseInt53", "premises"),
    ("premises", "premiseSha256", "premises"),
    ("premises", "premiseRecursionDepth", "premises"),
];

/// The families this crate does not answer, each with its reason and the
/// decision that records it.
const NOT_ANSWERED: &[(&str, &str, &str)] = &[
    (
        "encoding",
        "encodingProfile",
        "declined (D8): it pins the v1 rendering of stores the reference minted from 0.30.0 to \
         0.32.0, which the crate has never read or minted",
    ),
    (
        "identity",
        "identity",
        "not adopted (D14.7): a declared record's identity under its IDL vocabulary; the \
         vocabulary is a reference test fixture, not conformance data, two lines need the v1 \
         rendering D8 declined, and the crate holds no IDL vocabulary reader beyond the \
         canonical-form slice; a phase of its own when a consumer must mint a record's identity",
    ),
];

/// Every `(directory, family id)` the manifests under `conformance/` declare.
fn declared_families() -> BTreeSet<(String, String)> {
    let root = core_dir().join("conformance");
    let mut out = BTreeSet::new();
    let mut dirs: Vec<_> = fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("{}: {e}", root.display()))
        .map(|e| e.expect("a directory entry").path())
        .filter(|p| p.join("manifest.json").is_file())
        .collect();
    dirs.sort();
    for dir in dirs {
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .expect("a UTF-8 directory name")
            .to_string();
        let path = dir.join("manifest.json");
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let m = json::parse(&text).unwrap_or_else(|e| panic!("{}: {e:?}", path.display()));
        for f in arr(&m, "families") {
            assert!(
                out.insert((name.clone(), s(f, "id").to_string())),
                "{name}/manifest.json declares {} twice",
                s(f, "id")
            );
        }
    }
    out
}

#[test]
fn every_family_the_reference_emits_is_answered_or_named() {
    let declared = declared_families();
    let mut table = BTreeSet::new();
    for (dir, id, _) in ANSWERED.iter().chain(NOT_ANSWERED) {
        assert!(
            table.insert((dir.to_string(), id.to_string())),
            "{dir}/{id} is listed twice"
        );
    }
    let unlisted: Vec<_> = declared.difference(&table).collect();
    assert!(
        unlisted.is_empty(),
        "families the pinned Core emits that this crate neither answers nor names: {unlisted:?}"
    );
    let stale: Vec<_> = table.difference(&declared).collect();
    assert!(
        stale.is_empty(),
        "families listed here that the pinned Core no longer declares: {stale:?}"
    );
    for (dir, id, why) in NOT_ANSWERED {
        eprintln!("not answered: {dir}/{id} — {why}");
    }
}

#[test]
fn every_answering_leg_exists() {
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    for (dir, id, leg) in ANSWERED {
        assert!(
            here.join(format!("{leg}.rs")).is_file(),
            "{dir}/{id}: no leg tests/{leg}.rs"
        );
    }
}
