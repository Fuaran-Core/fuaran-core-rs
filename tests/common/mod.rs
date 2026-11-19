//! What every conformance leg shares: locating the Core checkout the vectors are
//! read from, and reading members out of a vector file. A leg that cannot find
//! its checkout, its file or a member it needs PANICS, naming what is missing.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use fuaran_core::capability::PolicyDecision;
use fuaran_core::column::Cell;
use fuaran_core::decode::DecodeError;
use fuaran_core::json::{self, JVal, PathSeg, render_canonical};

// ─── Locating Core ───────────────────────────────────────────────────────────

pub fn is_core(dir: &Path) -> bool {
    dir.join("conformance")
        .join("apply")
        .join("manifest.json")
        .is_file()
}

pub fn core_dir() -> PathBuf {
    if let Ok(d) = std::env::var("FUARAN_CORE_DIR") {
        let d = PathBuf::from(d);
        assert!(
            is_core(&d),
            "FUARAN_CORE_DIR={} is not a Core checkout (no conformance/apply/manifest.json)",
            d.display()
        );
        return d;
    }
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    for dir in here.ancestors().skip(1) {
        let candidate = dir.join("fuaran-core");
        if is_core(&candidate) {
            return candidate;
        }
    }
    panic!(
        "no Core checkout found: clone https://github.com/Fuaran-Core/fuaran-core beside this \
         repository (as fuaran-core/), checked out at the commit core-pin.json names, or set \
         FUARAN_CORE_DIR to one"
    );
}

pub fn read(rel: &str) -> JVal {
    let path = core_dir().join("conformance").join(rel);
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    json::parse(&text).unwrap_or_else(|e| panic!("{}: {e:?}", path.display()))
}

// ─── Reading the vector files ────────────────────────────────────────────────

pub fn get<'a>(v: &'a JVal, k: &str) -> &'a JVal {
    v.field(k)
        .unwrap_or_else(|| panic!("missing member '{k}' in {}", render_canonical(v)))
}

pub fn s<'a>(v: &'a JVal, k: &str) -> &'a str {
    match get(v, k) {
        JVal::Str(x) => x,
        other => panic!("'{k}' is not a string: {other:?}"),
    }
}

pub fn n(v: &JVal, k: &str) -> f64 {
    match get(v, k) {
        JVal::Num(x) => *x,
        other => panic!("'{k}' is not a number: {other:?}"),
    }
}

pub fn arr<'a>(v: &'a JVal, k: &str) -> &'a [JVal] {
    match get(v, k) {
        JVal::Arr(x) => x,
        other => panic!("'{k}' is not an array: {other:?}"),
    }
}

pub fn str_items(v: &JVal, k: &str) -> Vec<String> {
    arr(v, k)
        .iter()
        .map(|x| match x {
            JVal::Str(s) => s.clone(),
            other => panic!("not a string: {other:?}"),
        })
        .collect()
}

/// The family row's declared vector count in `<dir>/manifest.json`.
pub fn declared(dir: &str, family: &str) -> usize {
    let m = read(&format!("{dir}/manifest.json"));
    let row = arr(&m, "families")
        .iter()
        .find(|f| s(f, "id") == family)
        .unwrap_or_else(|| panic!("{dir}/manifest.json declares no {family}"));
    n(row, "vectors") as usize
}

pub fn vectors(doc: &JVal) -> &[JVal] {
    arr(doc, "vectors")
}

/// The leg's one line for a family it answers (DECISIONS.md D21):
/// `<family>: <answered> of <stated> vectors`, the number of vectors the leg
/// answered and the number the family's row in `<dir>/manifest.json` states.
/// The line is printed before it is judged, so a red log shows both numbers;
/// the leg fails when they differ.
pub fn answered(dir: &str, family: &str, answered: usize) {
    judge(dir, family, answered, String::new());
}

/// The line for a family answered by more than one binary, printed once, by the
/// binary that reads its manifest row: the number answered is the sum of every
/// binary's share, each named after it. A share answered elsewhere is one of
/// the constants below, which the answering binary asserts it ran, so the sum
/// holds exactly when every binary is green.
pub fn answered_across(dir: &str, family: &str, shares: &[(&str, usize)]) {
    let total = shares.iter().map(|(_, k)| k).sum();
    let parts: Vec<String> = shares.iter().map(|(leg, k)| format!("{leg} {k}")).collect();
    judge(dir, family, total, format!(" ({})", parts.join(", ")));
}

fn judge(dir: &str, family: &str, answered: usize, detail: String) {
    let stated = declared(dir, family);
    // One write, the newline included, so no test running beside this one can
    // split the line when the harness does not capture it.
    let line = format!(
        "{family}: {answered} of {stated} vectors{detail}
"
    );
    eprint!("{line}");
    assert_eq!(
        answered, stated,
        "{dir}/{family}: answered {answered} vectors{detail} where the manifest states {stated}"
    );
}

/// `codecRefusals`' column vectors, answered by tests/columnar.rs.
pub const CODEC_REFUSALS_COLUMN: usize = 26;
/// `codecRefusals`' profile vectors, answered by tests/propagation_forms.rs.
pub const CODEC_REFUSALS_PROFILE: usize = 19;
/// `decimal`'s column codec and aggregate vectors, answered by tests/columnar.rs.
pub const DECIMAL_COLUMN: usize = 22;

/// A refusal's path as the vectors write it: a member by its name, an item by
/// its index.
pub fn path_json(e: &DecodeError) -> Vec<JVal> {
    e.path
        .iter()
        .map(|s| match s {
            PathSeg::Key(k) => JVal::Str(k.clone()),
            PathSeg::Index(i) => JVal::Num(*i as f64),
        })
        .collect()
}

// ─── Shared shapes of the law files ──────────────────────────────────────────

/// A cell as the query law files write it: `{kind, text}`, no text for a null,
/// the text as it stands and never canonicalised; a float's text is its shortest
/// round-trip form, or a non-finite float's token in quotes (`"NaN"`), as the
/// canonical float layout writes it (DECISIONS.md D17.2).
pub fn cell_of(c: &JVal) -> Cell {
    let text = || s(c, "text").to_string();
    match s(c, "kind") {
        "int" => Cell::Int(text().parse().expect("an int's text")),
        "float" => Cell::Float(match s(c, "text") {
            "\"NaN\"" => f64::NAN,
            "\"Infinity\"" => f64::INFINITY,
            "\"-Infinity\"" => f64::NEG_INFINITY,
            t => t.parse().expect("a float's text"),
        }),
        "bool" => Cell::Bool(match s(c, "text") {
            "true" => true,
            "false" => false,
            other => panic!("a bool's text is {other}"),
        }),
        "string" => Cell::Str(text()),
        "date" => Cell::Date(text()),
        "timestamp" => Cell::Timestamp(text()),
        "decimal" => Cell::Decimal(text()),
        "null" => Cell::Null,
        other => panic!("unknown cell kind {other}"),
    }
}

/// A read's arguments as the query law files write them: `args`, each
/// `{name, cell}`.
pub fn args_of(input: &JVal) -> Vec<(String, Cell)> {
    arr(input, "args")
        .iter()
        .map(|a| (s(a, "name").to_string(), cell_of(get(a, "cell"))))
        .collect()
}

/// A gate rule's decision as the gate tables write it: `allow`, `needsApproval`,
/// or `deny` with its `message` and `alternatives`.
pub fn decision_of(v: &JVal) -> PolicyDecision {
    match s(v, "decision") {
        "allow" => PolicyDecision::Allow,
        "needsApproval" => PolicyDecision::NeedsApproval,
        "deny" => PolicyDecision::deny_with(
            s(v, "message"),
            str_items(v, "alternatives")
                .iter()
                .map(|a| Arc::from(a.as_str()))
                .collect::<Vec<Arc<str>>>(),
        ),
        other => panic!("unknown decision {other}"),
    }
}
