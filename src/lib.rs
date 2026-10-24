//! The Fuaran Core spine in Rust: the pieces every Rust host of a Fuaran wire
//! builds on, with nothing of any one wire's vocabulary in them.
//!
//! - [`json`] — the value model, the strict reader with its limits record, and
//!   the canonical writer;
//! - [`number`] — the canonical number form; [`decimal`] — the exact decimal;
//! - [`sha256`] and [`address`] — the digest and the content address;
//!   [`hash`] — the non-cryptographic FNV-1a fingerprint and field encoding;
//! - [`decode`] — the decode error contract and the reading combinators;
//! - [`tree`] and [`ops`] — the skeleton tree and its ops' apply;
//! - [`chain`] — the op-stream chain over opaque payloads, and the actor;
//! - [`dag`] — the content-addressed op-DAG, its orders, ranks and replays;
//!   [`merge`] — the N-lane reconcile; [`diff`] — the structural tree diff;
//! - [`column`], [`field`] and [`unit`] — the columnar codec and aggregates, the
//!   schema field, and the unit of measure's canonical text;
//! - [`propagation`] — change propagation over a declared dependency graph;
//! - [`capability`] — the invocable-capability contract; [`seam`] — the seams'
//!   refusal and declaration documents;
//! - [`canonical_form`] — an IDL vocabulary's canonical form; [`profile`] — the
//!   wire profile.
//!
//! Each is the twin of the reference implementation (`Fuaran.Core`) and is
//! certified against the conformance vectors it emits (`tests/`, one file per
//! group of families; DECISIONS.md D8 says which).
//!
//! `no_std` + `alloc` by default, so it builds for microcontroller targets; the
//! `std` feature adds nothing to the surface today and exists so a hosted
//! consumer can ask for it without a breaking change later.

#![no_std]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

/// What every module takes from `alloc` in place of the standard prelude.
#[allow(unused_imports)]
mod prelude {
    pub use alloc::borrow::ToOwned;
    pub use alloc::collections::{BTreeMap, BTreeSet};
    pub use alloc::format;
    pub use alloc::string::{String, ToString};
    pub use alloc::vec;
    pub use alloc::vec::Vec;
}

pub mod address;
pub mod canonical_form;
pub mod capability;
pub mod chain;
pub mod column;
pub mod dag;
pub mod decimal;
pub mod decode;
pub mod diff;
pub mod field;
pub mod hash;
pub mod json;
pub mod merge;
pub mod number;
pub mod ops;
pub mod profile;
pub mod propagation;
pub mod seam;
pub mod sha256;
pub mod tree;
pub mod unit;
