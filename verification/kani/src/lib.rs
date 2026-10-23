//! Kani harnesses over the spine's reader (`fuaran_core::json::read`).
//!
//! Each harness reads EVERY valid UTF-8 text of at most `N` bytes (the bytes and
//! the length are symbolic) under a symbolic limits record, and proves:
//!
//! * `read_is_panic_free` — the reader returns on every such input: no panic, no
//!   arithmetic overflow, no out-of-bounds index, no undefined behaviour.
//! * `read_respects_its_limits` — an accepted value holds no container, string or
//!   nesting past the bound it was read under; a breach names the bound's own
//!   declared value; an oversized document is refused before anything is read.
//! * `read_allocation_is_bounded` — the peak live heap of the read stays under a
//!   stated ceiling, measured by a counting global allocator, and a document past
//!   its `documentBytes` bound allocates nothing at all.
//!
//! See `../README.md` for the sizes proved, the cost and the verdict.

#[cfg(kani)]
mod harnesses;
