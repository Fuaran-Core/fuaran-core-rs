//! The Fuaran Core spine in Rust: the pieces every Rust host of a Fuaran wire
//! builds on, with nothing of any one wire's vocabulary in them.
//!
//! - [`json`] — the value model, the strict reader with its limits record, and
//!   the canonical writer;
//! - [`number`] — the canonical number form; [`decimal`] — the exact decimal;
//! - [`sha256`] and [`address`] — the digest and the content address;
//! - [`decode`] — the decode error contract and the reading combinators;
//! - [`tree`] and [`ops`] — the skeleton tree and its ops' apply;
//! - [`chain`] — the op-stream chain over opaque payloads, and the actor;
//! - [`dag`] — the content-addressed op-DAG, its orders, ranks and replays.
//!
//! Each is the twin of the reference implementation (`Fuaran.Core`) and is
//! certified against the conformance vectors it emits (`tests/conformance.rs`).
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
pub mod chain;
pub mod dag;
pub mod decimal;
pub mod decode;
pub mod json;
pub mod number;
pub mod ops;
pub mod sha256;
pub mod tree;
