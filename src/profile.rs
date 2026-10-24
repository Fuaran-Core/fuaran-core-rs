//! The wire profile: a capability id `<name>@<major>.<minor>` (`core@1.0`), the
//! value of an envelope's `$profile` member.
//!
//! The grammar is a BIJECTION between the valid profiles and their canonical
//! strings, so two strings never name one profile and a required profile can be
//! compared as text:
//!
//! ```text
//! profile = name "@" number "." number
//! name    = letter (letter | digit | "." | "_" | "-")*      ; ASCII only
//! number  = "0" | nonzero digit*                            ; at most 2147483647
//! ```
//!
//! [`try_parse`] accepts exactly the strings [`render`] writes. Twin of
//! `Versioning.Capability` (`Fuaran.Core.Wire`); certified against the `profile`
//! vectors of Core's `refusals/codec-refusals.json`, whose one refusal class is
//! `MalformedProfile`.

use crate::json::escape_string;
use crate::prelude::*;

/// A wire capability id. Twin of `Versioning.Capability`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// The capability namespace, a name of the grammar.
    pub name: String,
    /// The incompatibility boundary; non-negative on the wire.
    pub major: i32,
    /// The additive capability counter; non-negative on the wire.
    pub minor: i32,
}

/// Why a string is not a profile: always the reference's `MalformedProfile`
/// class, with a sentence naming the part that is outside the grammar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MalformedProfile {
    pub message: String,
}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic()
}

fn is_name_char(c: char) -> bool {
    is_name_start(c) || c.is_ascii_digit() || c == '.' || c == '_' || c == '-'
}

/// Whether `name` is a profile name of the grammar: an ASCII letter, then ASCII
/// letters, digits, `.`, `_` and `-`. Twin of `Capability.isValidName`.
pub fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if is_name_start(c) => chars.all(is_name_char),
        _ => false,
    }
}

/// Whether `p` is a profile the wire can carry. Twin of `Capability.isValid`.
pub fn is_valid(p: &Profile) -> bool {
    is_valid_name(&p.name) && p.major >= 0 && p.minor >= 0
}

/// The canonical string `<name>@<major>.<minor>`, assuming a valid profile.
/// Twin of `Capability.render`.
pub fn render(p: &Profile) -> String {
    format!("{}@{}.{}", p.name, p.major, p.minor)
}

/// [`render`], refusing a profile [`try_parse`] could not read back: a name
/// outside the grammar or a negative counter. Twin of `Capability.tryRender`.
pub fn try_render(p: &Profile) -> Result<String, MalformedProfile> {
    if !is_valid_name(&p.name) {
        Err(MalformedProfile {
            message: format!(
                "profile name is outside the grammar (an ASCII letter, then letters, digits, '.', '_' or '-'): {}",
                escape_string(&p.name)
            ),
        })
    } else if p.major < 0 || p.minor < 0 {
        Err(MalformedProfile {
            message: format!("profile version is negative: {}.{}", p.major, p.minor),
        })
    } else {
        Ok(render(p))
    }
}

/// A canonical number of the grammar: ASCII digits only, no sign, no leading
/// zero, within `i32`.
fn canonical_number(t: &str) -> Option<i32> {
    let b = t.as_bytes();
    if b.is_empty() || !b.iter().all(u8::is_ascii_digit) || (b.len() > 1 && b[0] == b'0') {
        return None;
    }
    t.parse::<i32>().ok()
}

/// Parse a profile string — the canonical form and nothing else. The name is
/// everything before the LAST `@`. Twin of `Capability.tryParse`.
pub fn try_parse(s: &str) -> Result<Profile, MalformedProfile> {
    let malformed = |message: String| Err(MalformedProfile { message });
    let at = match s.rfind('@') {
        Some(at) if at > 0 && at < s.len() - 1 => at,
        _ => {
            return malformed(format!(
                "malformed profile (expected '<name>@<major>.<minor>'): {s}"
            ));
        }
    };
    let (name, ver) = (&s[..at], &s[at + 1..]);
    if !is_valid_name(name) {
        return malformed(format!(
            "malformed profile name (expected an ASCII letter, then letters, digits, '.', '_' or '-'): {}",
            escape_string(name)
        ));
    }
    let parts: Vec<&str> = ver.split('.').collect();
    match parts.as_slice() {
        [maj, min] => match (canonical_number(maj), canonical_number(min)) {
            (Some(major), Some(minor)) => Ok(Profile {
                name: name.to_string(),
                major,
                minor,
            }),
            _ => malformed(format!(
                "malformed profile version (expected '<major>.<minor>', each a canonical non-negative integer within Int32): {}",
                escape_string(ver)
            )),
        },
        _ => malformed(format!(
            "malformed profile version (expected '<major>.<minor>'): {}",
            escape_string(ver)
        )),
    }
}
