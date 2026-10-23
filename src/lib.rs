//! The Fuaran Core spine in Rust.
//!
//! `no_std` + `alloc` by default; the `std` feature adds what only a hosted
//! target can offer. Phase 463 moves the spine here from the UI host crate.

#![no_std]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;
