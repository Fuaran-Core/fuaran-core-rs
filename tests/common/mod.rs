//! What every conformance leg shares: locating the Core checkout the vectors are
//! read from, and reading members out of a vector file. A leg that cannot find
//! its checkout, its file or a member it needs PANICS, naming what is missing.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};

use fuaran_core::json::{self, JVal, render_canonical};

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
