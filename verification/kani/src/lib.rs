//! Kani harnesses over the spine (`fuaran_core`).
//!
//! What closes, and runs under `run.ps1 -Verify`: `escape_len0` / `escape_len1` — the
//! canonical escape (`json::escape_string`) on the empty string and on every one-byte
//! ASCII string returns, is quoted, and is exactly as long as rule 6 says.
//!
//! What does not close on a 16 GB machine, kept behind the `frontier` feature with its
//! measurements: the escape on two and three symbolic bytes, and the rungs over the
//! limited reader — every valid UTF-8 text of at most N bytes under a symbolic limits
//! record: the reader returns, an accepted value is within every bound, a breach names
//! the bound it passed, and the peak live heap stays under a stated ceiling.
//!
//! See `../README.md` for the measurements, the cost and the verdict.

#[cfg(kani)]
mod harnesses;
