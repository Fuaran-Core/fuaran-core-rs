//! The text kernel's non-cryptographic fingerprint: 32-bit FNV-1a over UTF-16
//! code units, and the injective field encoding a multi-field pre-image is
//! built with. The twin of the FNV half of the reference's `Hash`
//! (`Fuaran.Core.Text`); the cryptographic half is [`crate::sha256`].
//!
//! FNV-1a is cheap and a second pre-image is seconds of search, so it belongs
//! on a cache key, a capture key or a schema fingerprint — never under a
//! signature.

use crate::prelude::*;
use core::fmt::{self, Write};

/// The field terminator of a canonical pre-image, `U+0001`. Twin of `Hash.foldSep`.
pub const FOLD_SEP: char = '\u{0001}';

/// The escape written before a [`FOLD_SEP`] or a [`FIELD_ESC`] a field carries,
/// `U+0010`. Twin of `Hash.fieldEsc`.
pub const FIELD_ESC: char = '\u{0010}';

/// The raw 32-bit FNV-1a over the UTF-16 CODE UNITS of `s` (not its bytes, not
/// its code points): each code unit folded whole, an astral character as its two
/// surrogates. Twin of `Hash.fnv1a32`.
pub fn fnv1a32(s: &str) -> u32 {
    s.encode_utf16().fold(2_166_136_261u32, |h, unit| {
        (h ^ u32::from(unit)).wrapping_mul(16_777_619)
    })
}

/// FNV-1a as eight lower-case hex digits. Twin of `Hash.fnv1a`.
pub fn fnv1a(s: &str) -> String {
    format!("{:08x}", fnv1a32(s))
}

/// One field of an injective pre-image: every escape and terminator it carries
/// escaped, then the terminator. Twin of `Hash.canonicalField`.
pub fn canonical_field(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 1);
    write_canonical_field(s, &mut out).expect("a String accepts every write");
    out
}

/// [`canonical_field`] written through `out`, building nothing.
pub(crate) fn write_canonical_field<W: Write + ?Sized>(s: &str, out: &mut W) -> fmt::Result {
    FieldBody(out).write_str(s)?;
    out.write_char(FOLD_SEP)
}

/// The body of one field as it is written: every [`FIELD_ESC`] and [`FOLD_SEP`]
/// written through it is preceded by [`FIELD_ESC`]; the caller writes the
/// terminator. A writer, so a field's text can be produced straight into the
/// pre-image (a number through `core::fmt`) without being spelled first.
pub(crate) struct FieldBody<'a, W: ?Sized>(pub(crate) &'a mut W);

impl<W: Write + ?Sized> Write for FieldBody<'_, W> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let mut from = 0;
        for (i, c) in s.char_indices() {
            if c == FIELD_ESC || c == FOLD_SEP {
                self.0.write_str(&s[from..i])?;
                self.0.write_char(FIELD_ESC)?;
                from = i;
            }
        }
        self.0.write_str(&s[from..])
    }
}

/// The canonical pre-image of a field sequence; injective. Twin of `Hash.canonicalFields`.
pub fn canonical_fields<S: AsRef<str>>(fields: &[S]) -> String {
    fields.iter().map(|f| canonical_field(f.as_ref())).collect()
}

/// The key builder: `prefix#fnv1a(canonical_fields(fields))`. Twin of `Hash.keyedBy`.
pub fn keyed_by<S: AsRef<str>>(prefix: &str, fields: &[S]) -> String {
    keyed_by_preimage(prefix, &canonical_fields(fields))
}

/// [`keyed_by`] over a pre-image already written: `prefix#fnv1a(preimage)`.
pub(crate) fn keyed_by_preimage(prefix: &str, preimage: &str) -> String {
    format!("{prefix}#{}", fnv1a(preimage))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fingerprint_folds_utf16_code_units() {
        assert_eq!(fnv1a(""), "811c9dc5");
        assert_eq!(fnv1a("a"), "e40c292c");
        // An astral character is two code units, not four bytes.
        assert_ne!(fnv1a("\u{1F600}"), fnv1a("\u{F600}"));
        assert_eq!(canonical_field("a\u{0001}b"), "a\u{0010}\u{0001}b\u{0001}");
    }
}
