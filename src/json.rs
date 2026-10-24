//! Canonical JSON: the value model, the strict reader every wire's JSON layer
//! cites, and the canonical writer.
//!
//! Every Fuaran wire is JSON below its own vocabulary, and the reference states
//! once what that JSON is (its "JSON-syntax layer"). A conformant document is one
//! RFC 8259 text, read strictly; each row below is a refusal of the TEXT, and no
//! conformant encoder can reach one:
//!
//! | Row | Input class | Refused as |
//! |---|---|---|
//! | 1 | a repeated member name in one object, compared after unescaping | [`JsonErrorKind::DuplicateMember`] |
//! | 2 | content after the root value | [`JsonErrorKind::TrailingCharacters`] |
//! | 3 | a number outside the RFC 8259 grammar | [`JsonErrorKind::MalformedNumber`] (`UnexpectedChar` where the token cannot start a value: `+1`, `.5`) |
//! | 4 | a bare `NaN` / `Infinity` / `-Infinity` | [`JsonErrorKind::UnexpectedChar`] (`MalformedNumber` for `-Infinity`, whose sign starts a number) |
//! | 5 | a raw C0 control character inside a string | [`JsonErrorKind::BadEscape`] |
//! | 6 | an unpaired surrogate, raw or escaped | [`JsonErrorKind::BadEscape`] |
//! | 7 | an overflowing exponent (`1e999`), or an integer token past 2^53 that is not the canonical layout of the double it reads as | [`JsonErrorKind::MalformedNumber`] |
//!
//! A quoted sentinel (`"NaN"`) is a string: what a typed float slot makes of it
//! is the citing wire's question, not this layer's.
//!
//! **Limits are a mechanism, never values.** Every read takes a [`JsonLimits`]
//! record and enforces each bound ON THE WAY DOWN, before the container, code
//! point, item or member it guards is read, so nothing past a bound is ever read
//! or built. A breach is [`JsonRefusal::LimitExceeded`] at the path of the
//! container it was raised in, and is never reported as malformed text; the first
//! fault in document order wins, whichever kind it is. The values belong to each
//! wire: a host passes its specification's numbers and maps the breach to its own
//! envelope. [`JsonLimits::STRICT`] bounds the depth at 512, the reference's cap,
//! and nothing else.
//!
//! The model and its answers are certified against Core's `refusals/` (the `json`
//! vectors) and `decode/` families.

use core::cmp::Ordering;
use core::marker::PhantomData;

use crate::number::format_number;
use crate::prelude::*;

mod escape;

/// A parsed JSON value. `Obj` keeps parse order (a reader looks members up by
/// name; [`render_canonical`] re-sorts on emit). A repeated member name never
/// reaches this type: row 1 refuses it at the reader.
#[derive(Debug, Clone, PartialEq)]
pub enum JVal {
    Null,
    Bool(bool),
    /// JSON numbers read as IEEE-754 doubles.
    ///
    /// That is conformant rather than a limitation, and the reasoning is worth
    /// keeping because the obvious remedy is unnecessary: integer identity on the
    /// wire stops at ±2⁵³, and the bound was chosen precisely because the integer
    /// and float canonical layouts AGREE over that range — a double holds every
    /// integer in it, and the fixed-point window of the number form (base-10
    /// exponent ≤ 16) covers every one of them, so re-encoding produces the
    /// integer spelling with no integer type in play. Past the bound row 7 admits
    /// only a token that IS the canonical layout of its double, so there is
    /// nothing an `i64` arm could preserve that a host is required to preserve.
    Num(f64),
    Str(String),
    Arr(Vec<JVal>),
    Obj(Vec<(String, JVal)>),
}

impl JVal {
    /// Member lookup by name, in any member order. A repeated name cannot occur —
    /// row 1 refuses it — so the first match is the only match. `None` on a
    /// non-object.
    pub fn field(&self, key: &str) -> Option<&JVal> {
        match self {
            JVal::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
}

// ─── The refusals ────────────────────────────────────────────────────────────

/// The class of a malformed text — the reference's closed `JsonErrorKind`, every
/// case of it, so a refusal names the class the reference names. The sentence
/// beside a refusal is this host's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonErrorKind {
    UnexpectedChar,
    UnexpectedEndOfInput,
    ExpectedToken,
    UnterminatedString,
    UnterminatedEscape,
    TruncatedUnicodeEscape,
    BadEscape,
    BadHexDigit,
    MalformedNumber,
    /// A `null` where the reading admits none. [`parse`] and [`read`] build
    /// [`JVal::Null`] and never raise it; the decode layer ([`crate::decode`])
    /// reads Core's null-free model and does.
    NullNotRepresentable,
    /// The reference's own name for its depth cap. This reader raises a depth
    /// breach as [`JsonRefusal::LimitExceeded`] with [`JsonLimit::Depth`], as the
    /// reference does under a limits record; the case is kept so the vocabulary is
    /// the reference's, whole.
    MaxDepthExceeded,
    TrailingCharacters,
    DuplicateMember,
}

impl JsonErrorKind {
    /// The reference's spelling of the class (`MalformedNumber`, …).
    pub fn as_str(self) -> &'static str {
        match self {
            JsonErrorKind::UnexpectedChar => "UnexpectedChar",
            JsonErrorKind::UnexpectedEndOfInput => "UnexpectedEndOfInput",
            JsonErrorKind::ExpectedToken => "ExpectedToken",
            JsonErrorKind::UnterminatedString => "UnterminatedString",
            JsonErrorKind::UnterminatedEscape => "UnterminatedEscape",
            JsonErrorKind::TruncatedUnicodeEscape => "TruncatedUnicodeEscape",
            JsonErrorKind::BadEscape => "BadEscape",
            JsonErrorKind::BadHexDigit => "BadHexDigit",
            JsonErrorKind::MalformedNumber => "MalformedNumber",
            JsonErrorKind::NullNotRepresentable => "NullNotRepresentable",
            JsonErrorKind::MaxDepthExceeded => "MaxDepthExceeded",
            JsonErrorKind::TrailingCharacters => "TrailingCharacters",
            JsonErrorKind::DuplicateMember => "DuplicateMember",
        }
    }
}

/// Malformed text: the class, the byte offset it was detected at, and a sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonError {
    pub kind: JsonErrorKind,
    pub offset: usize,
    pub message: String,
}

/// One step of a path from the document root: a member name or an item index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSeg {
    Key(String),
    Index(usize),
}

/// A bound of [`JsonLimits`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonLimit {
    /// Nested objects and arrays admitted, the root counting as one.
    Depth,
    /// Unicode code points in ONE decoded string or member name — a surrogate pair
    /// is one, whichever spelling it arrived in.
    StringLength,
    /// Items in one array.
    ArrayLength,
    /// Members in one object.
    ObjectMembers,
    /// UTF-8 bytes of the whole input.
    DocumentBytes,
}

impl JsonLimit {
    /// The reference's member name for the bound (`depth`, `stringLength`, …).
    pub fn as_str(self) -> &'static str {
        match self {
            JsonLimit::Depth => "depth",
            JsonLimit::StringLength => "stringLength",
            JsonLimit::ArrayLength => "arrayLength",
            JsonLimit::ObjectMembers => "objectMembers",
            JsonLimit::DocumentBytes => "documentBytes",
        }
    }
}

/// A bound passed: which, its declared value, the path of the container it was
/// raised in (the string's own path for a string value, the object's for a member
/// name, the root for the document size), and the byte offset. Never the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LimitBreach {
    pub limit: JsonLimit,
    pub value: usize,
    pub path: Vec<PathSeg>,
    pub offset: usize,
}

/// Why a read refused: the text is malformed, or it is well-formed and passes a
/// bound. The two are never conflated — a breach reported as malformed sends the
/// author to repair the wrong thing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonRefusal {
    Malformed(JsonError),
    LimitExceeded(LimitBreach),
}

impl JsonRefusal {
    /// The byte offset the refusal was raised at.
    pub fn offset(&self) -> usize {
        match self {
            JsonRefusal::Malformed(e) => e.offset,
            JsonRefusal::LimitExceeded(b) => b.offset,
        }
    }
}

/// The bounds a read enforces on the way down. `None` is unbounded; the depth is
/// always bounded, because it sizes the reader's nesting stack: at most one frame
/// per level, allocated once, on the heap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JsonLimits {
    pub depth: usize,
    pub string_length: Option<usize>,
    pub array_length: Option<usize>,
    pub object_members: Option<usize>,
    pub document_bytes: Option<usize>,
}

impl JsonLimits {
    /// The reference's depth cap: nesting past 512 is refused, and no other
    /// bound applies. Deliberately looser than any wire's bound, so the reader
    /// never refuses a document a wire says a host must accept.
    pub const STRICT: JsonLimits = JsonLimits::depth_only(512);

    /// A record bounding the depth alone.
    pub const fn depth_only(depth: usize) -> JsonLimits {
        JsonLimits {
            depth,
            string_length: None,
            array_length: None,
            object_members: None,
            document_bytes: None,
        }
    }
}

impl Default for JsonLimits {
    fn default() -> Self {
        JsonLimits::STRICT
    }
}

type PResult<T> = Result<T, JsonRefusal>;

// ─── The reader ──────────────────────────────────────────────────────────────

/// What a reading builds from the tokens. One reader serves both value models —
/// [`JVal`] here and the null-free, integer-aware model of [`crate::decode`] — so
/// the grammar, the rows and the limits are stated once.
pub(crate) trait Build {
    type V;
    /// `None` when the reading admits no `null` ([`JsonErrorKind::NullNotRepresentable`]).
    fn null() -> Option<Self::V>;
    fn boolean(b: bool) -> Self::V;
    /// `integer_token` is true for a token with no point and no exponent.
    fn number(n: f64, integer_token: bool) -> Self::V;
    fn string(s: String) -> Self::V;
    fn array(items: Vec<Self::V>) -> Self::V;
    fn object(members: Vec<(String, Self::V)>) -> Self::V;
}

struct JValBuild;

impl Build for JValBuild {
    type V = JVal;
    fn null() -> Option<JVal> {
        Some(JVal::Null)
    }
    fn boolean(b: bool) -> JVal {
        JVal::Bool(b)
    }
    fn number(n: f64, _: bool) -> JVal {
        JVal::Num(n)
    }
    fn string(s: String) -> JVal {
        JVal::Str(s)
    }
    fn array(items: Vec<JVal>) -> JVal {
        JVal::Arr(items)
    }
    fn object(members: Vec<(String, JVal)>) -> JVal {
        JVal::Obj(members)
    }
}

struct Parser<'a, B> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    limits: JsonLimits,
    build: PhantomData<B>,
}

/// One open container on the reader's nesting stack: what has been read of it so
/// far. The stack is the reader's whole nesting state, so the native stack a read
/// uses is the same at every depth.
enum Frame<V> {
    Array(Vec<V>),
    Object {
        members: Vec<(String, V)>,
        /// The names seen so far: a set keeps the row-1 refusal linear in width,
        /// where scanning every earlier member made a wide object quadratic.
        seen: BTreeSet<String>,
        /// The name of the member whose value is being read, and the offset of
        /// its opening quote.
        key: String,
        key_at: usize,
    },
}

impl<V> Frame<V> {
    /// The step from this container to the child being read in it.
    fn step(&self) -> PathSeg {
        match self {
            Frame::Array(items) => PathSeg::Index(items.len()),
            Frame::Object { key, .. } => PathSeg::Key(key.clone()),
        }
    }
}

impl<'a, B: Build> Parser<'a, B> {
    fn fail<T>(&self, kind: JsonErrorKind, message: impl Into<String>) -> PResult<T> {
        self.fail_at(kind, self.pos, message)
    }

    fn fail_at<T>(
        &self,
        kind: JsonErrorKind,
        offset: usize,
        message: impl Into<String>,
    ) -> PResult<T> {
        Err(JsonRefusal::Malformed(JsonError {
            kind,
            offset,
            message: message.into(),
        }))
    }

    /// A bound passed in the innermost of the `open` containers, at the path they
    /// spell from the root. Only a breach carries a path; malformed text is refused
    /// at the root by every reading.
    fn breach<T>(&self, limit: JsonLimit, value: usize, open: &[Frame<B::V>]) -> PResult<T> {
        Err(JsonRefusal::LimitExceeded(LimitBreach {
            limit,
            value,
            path: open.iter().map(Frame::step).collect(),
            offset: self.pos,
        }))
    }

    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b' ' | b'\t' | b'\n' | b'\r' => self.pos += 1,
                _ => break,
            }
        }
    }

    fn expect(&mut self, ch: u8) -> PResult<()> {
        match self.bytes.get(self.pos) {
            Some(&c) if c == ch => {
                self.pos += 1;
                Ok(())
            }
            Some(&c) => self.fail(
                JsonErrorKind::ExpectedToken,
                format!("expected '{}' but found '{}'", ch as char, c as char),
            ),
            None => self.fail(
                JsonErrorKind::UnexpectedEndOfInput,
                format!("expected '{}' but the input ended", ch as char),
            ),
        }
    }

    /// One string literal. Three rules are enforced HERE, on the way down, and
    /// each has to be:
    ///
    /// * **Row 6 — an unpaired surrogate is refused.** A Rust `String` holds
    ///   Unicode scalar values, so by the time the literal is assembled the
    ///   evidence is gone; and even on a UTF-16 host an assembled string cannot
    ///   tell a pair from two lone halves. A high half must be followed
    ///   IMMEDIATELY by a low half.
    /// * **Row 5 — a raw C0 control character is refused.** A conformant encoder
    ///   escapes them, so passing one through admits input no encoder produces.
    /// * **The string bound, counted in CODE POINTS as the literal is built.**
    ///   Measuring afterwards has already paid the allocation the bound exists to
    ///   refuse, and counting bytes would make the allowance depend on the
    ///   alphabet the author writes in.
    ///
    /// `open` is the containers the string is read in, for a breach's path: every
    /// open one for a string value, all but the object itself for a member name.
    fn parse_string(&mut self, open: &[Frame<B::V>]) -> PResult<String> {
        self.expect(b'"')?;
        let mut out = String::new();
        let mut code_points: usize = 0;
        let bound = self.limits.string_length;
        // Called before each code point is appended: the breach is raised at the
        // code point that would pass the bound.
        let admit = |p: &Self, count: &mut usize| -> PResult<()> {
            if let Some(max) = bound {
                if *count >= max {
                    return p.breach(JsonLimit::StringLength, max, open);
                }
            }
            *count += 1;
            Ok(())
        };
        loop {
            let Some(&c) = self.bytes.get(self.pos) else {
                return self.fail(JsonErrorKind::UnterminatedString, "unterminated string");
            };
            match c {
                b'"' => {
                    self.pos += 1;
                    return Ok(out);
                }
                b'\\' => {
                    let Some(&esc) = self.bytes.get(self.pos + 1) else {
                        return self.fail(JsonErrorKind::UnterminatedEscape, "unterminated escape");
                    };
                    let simple = match esc {
                        b'"' => Some('"'),
                        b'\\' => Some('\\'),
                        b'/' => Some('/'),
                        b'b' => Some('\u{0008}'),
                        b'f' => Some('\u{000C}'),
                        b'n' => Some('\n'),
                        b'r' => Some('\r'),
                        b't' => Some('\t'),
                        b'u' => None,
                        other => {
                            return self.fail(
                                JsonErrorKind::BadEscape,
                                format!("unknown escape '\\{}'", other as char),
                            );
                        }
                    };
                    admit(self, &mut code_points)?;
                    self.pos += 2;
                    match simple {
                        Some(ch) => out.push(ch),
                        None => {
                            let unit = self.parse_hex4()?;
                            let scalar = self.resolve_escape_unit(unit)?;
                            out.push(scalar);
                        }
                    }
                }
                0x00..=0x1F => {
                    return self.fail(
                        JsonErrorKind::BadEscape,
                        format!(
                            "a raw control character U+{c:04X} inside a string (the JSON-syntax \
                             layer, row 5): it must be escaped, and a conformant encoder escapes it"
                        ),
                    );
                }
                _ => {
                    admit(self, &mut code_points)?;
                    let ch = self.src[self.pos..]
                        .chars()
                        .next()
                        .expect("the position is on a character boundary of a non-empty rest");
                    out.push(ch);
                    self.pos += ch.len_utf8();
                }
            }
        }
    }

    /// Resolve one `\uXXXX` escape to the scalar it denotes, consuming a following
    /// low half when this one is a high half (row 6). Called with `self.pos` just
    /// past the four hex digits.
    fn resolve_escape_unit(&mut self, unit: u16) -> PResult<char> {
        if (0xD800..=0xDBFF).contains(&unit) {
            // "Immediately" is the whole content of the rule: a host that merely
            // counts surrogates reassembles a scalar the author never wrote out of
            // two halves that happened to co-occur.
            let has_low = self.bytes.get(self.pos) == Some(&b'\\')
                && self.bytes.get(self.pos + 1) == Some(&b'u');
            if !has_low {
                return self.fail_surrogate("high", unit);
            }
            self.pos += 2;
            let low = self.parse_hex4()?;
            if !(0xDC00..=0xDFFF).contains(&low) {
                return self.fail_surrogate("high", unit);
            }
            let combined = 0x10000 + ((u32::from(unit) - 0xD800) << 10) + (u32::from(low) - 0xDC00);
            return Ok(char::from_u32(combined).expect("a paired surrogate is a valid scalar"));
        }
        if (0xDC00..=0xDFFF).contains(&unit) {
            // A low half is only ever consumed above, as the second of a pair, so
            // reaching it here means it stands alone.
            return self.fail_surrogate("low", unit);
        }
        Ok(char::from_u32(u32::from(unit)).expect("a non-surrogate BMP unit is a valid scalar"))
    }

    fn fail_surrogate<T>(&self, half: &str, unit: u16) -> PResult<T> {
        self.fail(
            JsonErrorKind::BadEscape,
            format!(
                "an unpaired {half} surrogate escape \\u{unit:04X} (the JSON-syntax layer, row 6): \
                 a high half must be followed immediately by a low half"
            ),
        )
    }

    fn parse_hex4(&mut self) -> PResult<u16> {
        if self.pos + 4 > self.bytes.len() {
            return self.fail(
                JsonErrorKind::TruncatedUnicodeEscape,
                "truncated \\u escape",
            );
        }
        let mut value: u16 = 0;
        for i in 0..4 {
            let b = self.bytes[self.pos + i];
            let digit = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                b'A'..=b'F' => b - b'A' + 10,
                _ => {
                    return self.fail_at(
                        JsonErrorKind::BadHexDigit,
                        self.pos + i,
                        "bad hex digit in a \\u escape",
                    );
                }
            };
            value = value * 16 + u16::from(digit);
        }
        self.pos += 4;
        Ok(value)
    }

    /// One number token, held to the **RFC 8259 grammar before the platform
    /// reader** (row 3). Asking `str::parse::<f64>()` "is this a number" asks about
    /// Rust, not about this format: it accepts `+1`, `.5`, `5.`, `01`, `NaN` and
    /// `inf`, none of which RFC 8259 permits. Then the two VALUE rules of row 7: a
    /// value outside the finite double range is refused, and an integer token past
    /// 2^53 is read only when it is the canonical layout of the double it reads as
    /// — judged on the TOKEN, since 2^53 + 1 reads as 2^53.
    fn parse_number(&mut self) -> PResult<B::V> {
        let start = self.pos;
        while self.pos < self.bytes.len() && is_number_byte(self.bytes[self.pos]) {
            self.pos += 1;
        }
        let slice = &self.src[start..self.pos];
        if !is_rfc8259_number(slice) {
            return self.fail_at(
                JsonErrorKind::MalformedNumber,
                start,
                format!("malformed number: '{slice}' is not in the RFC 8259 grammar"),
            );
        }
        let integer_token = !slice.contains(['.', 'e', 'E']);
        match slice.parse::<f64>() {
            Ok(n) if !n.is_finite() => self.fail_at(
                JsonErrorKind::MalformedNumber,
                start,
                format!("malformed number: '{slice}' is outside the finite double range"),
            ),
            Ok(n) if integer_token && !int53_safe_or_canonical(slice, n) => self.fail_at(
                JsonErrorKind::MalformedNumber,
                start,
                format!(
                    "malformed number: the integer '{slice}' is past 2^53 and is not the canonical \
                     layout of a double, so it cannot round-trip"
                ),
            ),
            Ok(n) => Ok(B::number(n, integer_token)),
            Err(_) => self.fail_at(
                JsonErrorKind::MalformedNumber,
                start,
                format!("malformed number: '{slice}'"),
            ),
        }
    }

    fn literal(&mut self, word: &str, value: impl FnOnce() -> PResult<B::V>) -> PResult<B::V> {
        if self.src[self.pos..].starts_with(word) {
            self.pos += word.len();
            value()
        } else {
            self.fail(JsonErrorKind::ExpectedToken, format!("expected '{word}'"))
        }
    }

    /// The document's one value, read without recursion: the containers open
    /// around the current position are an explicit stack, so a read uses the same
    /// native stack at every depth, and a depth bound is a property of the limits
    /// record rather than of the thread the read happens to run on.
    ///
    /// The stack lives in ONE allocation, made when the first container opens and
    /// sized to hold every container the read can have open at once: the depth
    /// bound, or the opening brackets left in the document when there are fewer
    /// (each open container consumed one). Nothing on the parse path grows it.
    fn parse_document(&mut self) -> PResult<B::V> {
        let mut open: Vec<Frame<B::V>> = Vec::new();
        'value: loop {
            // One value. A scalar is complete at once; a container that does not
            // close at once is pushed, and the loop goes round for its first child.
            self.skip_ws();
            let Some(&c) = self.bytes.get(self.pos) else {
                return self.fail(
                    JsonErrorKind::UnexpectedEndOfInput,
                    "unexpected end of input",
                );
            };
            let mut done = match c {
                b'{' | b'[' => {
                    // Refused BEFORE the container opens: a check after the walk
                    // has already paid the cost it exists to refuse.
                    if open.len() >= self.limits.depth {
                        return self.breach(JsonLimit::Depth, self.limits.depth, &open);
                    }
                    if open.capacity() == 0 {
                        let brackets = self.bytes[self.pos..]
                            .iter()
                            .filter(|&&b| b == b'{' || b == b'[')
                            .count();
                        open = Vec::with_capacity(self.limits.depth.min(brackets));
                    }
                    self.pos += 1;
                    self.skip_ws();
                    let empty = if c == b'[' { b']' } else { b'}' };
                    if self.bytes.get(self.pos) == Some(&empty) {
                        self.pos += 1;
                        if c == b'[' {
                            B::array(Vec::new())
                        } else {
                            B::object(Vec::new())
                        }
                    } else {
                        let mut frame = if c == b'[' {
                            Frame::Array(Vec::new())
                        } else {
                            Frame::Object {
                                members: Vec::new(),
                                seen: BTreeSet::new(),
                                key: String::new(),
                                key_at: 0,
                            }
                        };
                        self.next_child(&mut frame, &open)?;
                        debug_assert!(open.len() < open.capacity(), "the arena never grows");
                        open.push(frame);
                        continue 'value;
                    }
                }
                b'"' => B::string(self.parse_string(&open)?),
                b't' => self.literal("true", || Ok(B::boolean(true)))?,
                b'f' => self.literal("false", || Ok(B::boolean(false)))?,
                b'n' => {
                    let at = self.pos;
                    self.literal("null", || {
                        B::null().ok_or(JsonRefusal::Malformed(JsonError {
                            kind: JsonErrorKind::NullNotRepresentable,
                            offset: at,
                            message: "null is not representable in this reading".to_string(),
                        }))
                    })?
                }
                b'-' | b'0'..=b'9' => self.parse_number()?,
                other => {
                    return self.fail(
                        JsonErrorKind::UnexpectedChar,
                        format!("unexpected character '{}'", other as char),
                    );
                }
            };
            // Hand the finished value to the container it was read in, closing
            // each container it completes, until one wants another child or the
            // root is done.
            loop {
                let Some((frame, outer)) = open.split_last_mut() else {
                    return Ok(done);
                };
                let close = match frame {
                    Frame::Array(items) => {
                        items.push(done);
                        b']'
                    }
                    Frame::Object {
                        members,
                        seen,
                        key,
                        key_at,
                    } => {
                        // Row 1 — refused when the member is accumulated, after its
                        // value is read, at the repeated name's opening quote. It
                        // changes what a document MEANS rather than whether it is
                        // accepted: hosts kept the first or the last occurrence, so
                        // identical bytes read as different documents with no error
                        // raised anywhere.
                        if !seen.insert(key.clone()) {
                            return self.fail_at(
                                JsonErrorKind::DuplicateMember,
                                *key_at,
                                format!(
                                    "the member '{key}' appears more than once in one object \
                                     (the JSON-syntax layer, row 1)"
                                ),
                            );
                        }
                        members.push((core::mem::take(key), done));
                        b'}'
                    }
                };
                self.skip_ws();
                match self.bytes.get(self.pos) {
                    Some(b',') => {
                        self.pos += 1;
                        self.next_child(frame, outer)?;
                        continue 'value;
                    }
                    Some(&b) if b == close => {
                        self.pos += 1;
                        done = match open.pop() {
                            Some(Frame::Array(items)) => B::array(items),
                            Some(Frame::Object { members, .. }) => B::object(members),
                            None => unreachable!("the container just closed is on the stack"),
                        };
                    }
                    Some(&other) => {
                        return self.fail(
                            JsonErrorKind::ExpectedToken,
                            format!(
                                "expected ',' or '{}' but found '{}'",
                                close as char, other as char
                            ),
                        );
                    }
                    None => {
                        return self.fail(
                            JsonErrorKind::UnexpectedEndOfInput,
                            "unexpected end of input",
                        );
                    }
                }
            }
        }
    }

    /// Up to where the next child of `frame` starts: for an array, the item bound,
    /// checked before a character of the item is read; for an object, the member
    /// bound, then the member's name and its `:`. `outer` is the containers around
    /// `frame`, which spell a breach's path.
    fn next_child(&mut self, frame: &mut Frame<B::V>, outer: &[Frame<B::V>]) -> PResult<()> {
        match frame {
            Frame::Array(items) => {
                if let Some(max) = self.limits.array_length {
                    if items.len() >= max {
                        self.skip_ws();
                        return self.breach(JsonLimit::ArrayLength, max, outer);
                    }
                }
            }
            Frame::Object {
                members,
                key,
                key_at,
                ..
            } => {
                self.skip_ws();
                if let Some(max) = self.limits.object_members {
                    if members.len() >= max {
                        return self.breach(JsonLimit::ObjectMembers, max, outer);
                    }
                }
                *key_at = self.pos;
                *key = self.parse_string(outer)?;
                self.skip_ws();
                self.expect(b':')?;
            }
        }
        Ok(())
    }
}

fn is_number_byte(b: u8) -> bool {
    matches!(b, b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
}

/// An integer token within 2^53 in magnitude, or one past it that is the
/// canonical float layout of `n`, the double it reads as. Every other integer
/// token past 2^53 is refused (row 7).
fn int53_safe_or_canonical(slice: &str, n: f64) -> bool {
    let digits = slice.strip_prefix('-').unwrap_or(slice);
    let safe = digits.len() < 16 || (digits.len() == 16 && digits <= "9007199254740992");
    safe || format_number(n) == slice
}

/// The RFC 8259 number production, exactly:
///
/// ```text
/// number = [ "-" ] int [ frac ] [ exp ]
/// int    = "0" / ( digit1-9 *DIGIT )
/// frac   = "." 1*DIGIT
/// exp    = ("e" / "E") [ "+" / "-" ] 1*DIGIT
/// ```
///
/// Written out rather than delegated, because delegating is the defect: every
/// platform's float reader accepts a different superset, so a host that reaches
/// for one accepts a set that is a property of its runtime rather than of JSON.
fn is_rfc8259_number(s: &str) -> bool {
    let b = s.as_bytes();
    let mut i = 0usize;
    if b.first() == Some(&b'-') {
        i += 1;
    }
    match b.get(i).copied() {
        Some(b'0') => i += 1,
        Some(d) if d.is_ascii_digit() => {
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
        }
        _ => return false,
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    i == b.len()
}

/// The whole document under `limits`: the document bound before anything is
/// read, then exactly ONE value and nothing after it but whitespace (row 2).
pub(crate) fn scan<B: Build>(input: &str, limits: &JsonLimits) -> PResult<B::V> {
    if let Some(max) = limits.document_bytes {
        if input.len() > max {
            return Err(JsonRefusal::LimitExceeded(LimitBreach {
                limit: JsonLimit::DocumentBytes,
                value: max,
                path: Vec::new(),
                offset: 0,
            }));
        }
    }
    let mut p: Parser<'_, B> = Parser {
        src: input,
        bytes: input.as_bytes(),
        pos: 0,
        limits: *limits,
        build: PhantomData,
    };
    let value = p.parse_document()?;
    p.skip_ws();
    if p.pos < p.bytes.len() {
        return p.fail(
            JsonErrorKind::TrailingCharacters,
            "content after the root value (the JSON-syntax layer, row 2)",
        );
    }
    Ok(value)
}

/// Read a document under the reference's depth cap ([`JsonLimits::STRICT`]).
pub fn parse(input: &str) -> Result<JVal, JsonRefusal> {
    scan::<JValBuild>(input, &JsonLimits::STRICT)
}

/// Read a document under `limits`, each bound enforced on the way down.
pub fn read(input: &str, limits: &JsonLimits) -> Result<JVal, JsonRefusal> {
    scan::<JValBuild>(input, limits)
}

// ─── The canonical writer ────────────────────────────────────────────────────

/// Quote and escape a string the one canonical way: `"` and `\` escaped, every
/// control character U+0000–U+001F as a lower-case `\u00xx` with no short form,
/// and nothing else — `/` and non-ASCII pass through literally.
///
/// The work is [`escape::escape_bytes`] over the string's UTF-8 bytes, the function
/// the Creusot proof checks (`verification/README.md`). The text it returns is
/// valid UTF-8 by construction rather than by proof: the escape replaces only ASCII
/// bytes, each with ASCII ones, and copies every other byte in order. The check below
/// is one pass the crate keeps in place of an `unsafe` conversion it nowhere else
/// needs, and its failing arm is unreachable.
pub fn escape_string(s: &str) -> String {
    match String::from_utf8(escape::escape_bytes(s.as_bytes())) {
        Ok(quoted) => quoted,
        Err(_) => unreachable!("the canonical escape of UTF-8 text is UTF-8 text"),
    }
}

/// Ordinal comparison on UTF-16 code units — the reference's `StringComparer.Ordinal`,
/// the member order every host sorts by. It differs from Rust's `str` order
/// (Unicode scalar values) only above the BMP, which is exactly where a host that
/// used the default would silently diverge.
pub fn ordinal_cmp(a: &str, b: &str) -> Ordering {
    a.encode_utf16().cmp(b.encode_utf16())
}

/// An object from `(name, rendered value)` pairs: names sorted ordinally, no
/// whitespace. Each value is passed through verbatim.
pub fn render_object(fields: &mut [(String, String)]) -> String {
    fields.sort_by(|a, b| ordinal_cmp(&a.0, &b.0));
    let mut out = String::from("{");
    for (i, (k, v)) in fields.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&escape_string(k));
        out.push(':');
        out.push_str(v);
    }
    out.push('}');
    out
}

/// An array from rendered items, in source order.
pub fn render_array(items: &[String]) -> String {
    let mut out = String::from("[");
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(item);
    }
    out.push(']');
    out
}

/// A value's canonical bytes: names sorted ordinally, the canonical number form,
/// the canonical escapes. For a canonical input, `render_canonical(&parse(x)?) == x`.
///
/// Written into one buffer without recursion, as the reader reads: the containers
/// open around the value being written are an explicit stack, so writing a value
/// back uses the same native stack at every depth.
pub fn render_canonical(v: &JVal) -> String {
    /// An open container: the children still to write, and whether one has been
    /// written (so the next is preceded by a comma).
    enum Open<'a> {
        Arr(core::slice::Iter<'a, JVal>, bool),
        Obj(vec::IntoIter<&'a (String, JVal)>, bool),
    }
    let mut out = String::new();
    let mut open: Vec<Open<'_>> = Vec::new();
    let mut next = v;
    loop {
        match next {
            JVal::Null => out.push_str("null"),
            JVal::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            JVal::Num(n) => out.push_str(&format_number(*n)),
            JVal::Str(s) => out.push_str(&escape_string(s)),
            JVal::Arr(items) => {
                out.push('[');
                open.push(Open::Arr(items.iter(), false));
            }
            JVal::Obj(fields) => {
                out.push('{');
                let mut sorted: Vec<&(String, JVal)> = fields.iter().collect();
                sorted.sort_by(|a, b| ordinal_cmp(&a.0, &b.0));
                open.push(Open::Obj(sorted.into_iter(), false));
            }
        }
        // The next child of the innermost open container, closing each container
        // that has none left.
        loop {
            let Some(top) = open.last_mut() else {
                return out;
            };
            let child = match top {
                Open::Arr(items, any) => items.next().inspect(|_| {
                    if core::mem::replace(any, true) {
                        out.push(',');
                    }
                }),
                Open::Obj(members, any) => members.next().map(|(name, item)| {
                    if core::mem::replace(any, true) {
                        out.push(',');
                    }
                    out.push_str(&escape_string(name));
                    out.push(':');
                    item
                }),
            };
            match child {
                Some(item) => {
                    next = item;
                    break;
                }
                None => {
                    out.push(match open.pop() {
                        Some(Open::Arr(..)) => ']',
                        _ => '}',
                    });
                }
            }
        }
    }
}

/// The reader, run over a value model a bounded model checker can afford. Built
/// only under `cfg(kani)`; the harnesses live in `verification/kani/`.
///
/// It is the SAME reader [`read`] runs — [`scan`], every grammar row and every
/// bound — with [`Build`] instantiated at [`kani_support::Shape`], which keeps of a
/// value only the four measures the limits guard. [`JVal`]'s recursive drop glue is
/// what makes the public entry intractable to a model checker past a few bytes:
/// every drop site unrolls the recursion to the full unwinding bound whatever the
/// value's real depth, and the model grows exponentially in it.
#[cfg(kani)]
pub mod kani_support {
    use super::{Build, JsonLimits, JsonRefusal, scan};
    use crate::prelude::*;

    /// What a read built, reduced to the measure each bound guards: the nesting
    /// depth, and the largest string (in code points), array and object in it.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct Shape {
        pub depth: usize,
        pub string_length: usize,
        pub array_length: usize,
        pub object_members: usize,
    }

    impl Shape {
        fn join(self, other: Shape) -> Shape {
            Shape {
                depth: self.depth.max(other.depth),
                string_length: self.string_length.max(other.string_length),
                array_length: self.array_length.max(other.array_length),
                object_members: self.object_members.max(other.object_members),
            }
        }
    }

    /// Code points of a `str`: the bytes that do not continue a UTF-8 sequence. A
    /// plain byte loop, where `chars().count()` is a chunked word-at-a-time loop the
    /// model checker unrolls far less cheaply.
    fn code_points(s: &str) -> usize {
        s.bytes().filter(|b| b & 0xC0 != 0x80).count()
    }

    struct ShapeBuild;

    impl Build for ShapeBuild {
        type V = Shape;
        fn null() -> Option<Shape> {
            Some(Shape::default())
        }
        fn boolean(_: bool) -> Shape {
            Shape::default()
        }
        fn number(_: f64, _: bool) -> Shape {
            Shape::default()
        }
        fn string(s: String) -> Shape {
            Shape {
                string_length: code_points(&s),
                ..Shape::default()
            }
        }
        fn array(items: Vec<Shape>) -> Shape {
            let inner = items.iter().fold(Shape::default(), |a, s| a.join(*s));
            Shape {
                depth: inner.depth + 1,
                array_length: inner.array_length.max(items.len()),
                ..inner
            }
        }
        fn object(members: Vec<(String, Shape)>) -> Shape {
            let inner = members.iter().fold(Shape::default(), |a, (k, s)| {
                let name = Shape {
                    string_length: code_points(k),
                    ..Shape::default()
                };
                a.join(*s).join(name)
            });
            Shape {
                depth: inner.depth + 1,
                object_members: inner.object_members.max(members.len()),
                ..inner
            }
        }
    }

    /// [`super::read`], building a [`Shape`] in place of a [`super::JVal`].
    pub fn read_shape(input: &str, limits: &JsonLimits) -> Result<Shape, JsonRefusal> {
        scan::<ShapeBuild>(input, limits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(input: &str) -> JsonErrorKind {
        match parse(input) {
            Err(JsonRefusal::Malformed(e)) => e.kind,
            other => panic!("expected {input:?} to be malformed, got {other:?}"),
        }
    }

    #[test]
    fn empty_and_truncated_input_is_refused() {
        assert_eq!(kind(""), JsonErrorKind::UnexpectedEndOfInput);
        assert_eq!(kind("   "), JsonErrorKind::UnexpectedEndOfInput);
        assert_eq!(kind("{\"a\":"), JsonErrorKind::UnexpectedEndOfInput);
        assert_eq!(kind("not json"), JsonErrorKind::ExpectedToken);
        assert_eq!(kind("\"abc"), JsonErrorKind::UnterminatedString);
        assert_eq!(kind("\"\\"), JsonErrorKind::UnterminatedEscape);
        assert_eq!(kind("\"\\u12"), JsonErrorKind::TruncatedUnicodeEscape);
        assert_eq!(kind("\"\\u12g4\""), JsonErrorKind::BadHexDigit);
    }

    #[test]
    fn canonical_round_trip_is_byte_stable() {
        for case in [
            "{}",
            "[]",
            "{\"a\":1,\"b\":[true,false,null],\"c\":\"x\"}",
            "{\"$type\":\"Static\",\"value\":\"<opaque>\"}",
            "{\"n\":0.30000000000000004,\"z\":1E+21}",
            "\"a\\\\b\\\"c\\u0001\"",
        ] {
            let parsed = parse(case).expect(case);
            assert_eq!(render_canonical(&parsed), case, "round-trip of {case}");
        }
    }

    #[test]
    fn names_sort_ordinally_on_render() {
        let parsed = parse("{\"b\":1,\"a\":2,\"$type\":\"X\"}").unwrap();
        assert_eq!(
            render_canonical(&parsed),
            "{\"$type\":\"X\",\"a\":2,\"b\":1}"
        );
        // Above the BMP the UTF-16 order is not the scalar order.
        assert_eq!(ordinal_cmp("\u{FFFF}", "\u{10000}"), Ordering::Greater);
    }

    #[test]
    fn the_rows_are_refused_with_their_classes() {
        assert_eq!(kind("{\"a\":1,\"a\":2}"), JsonErrorKind::DuplicateMember);
        assert_eq!(
            kind("{\"o\":{\"a\":1,\"a\":2}}"),
            JsonErrorKind::DuplicateMember
        );
        assert!(parse("[{\"a\":1},{\"a\":2}]").is_ok());
        assert_eq!(kind("{} {}"), JsonErrorKind::TrailingCharacters);
        assert_eq!(kind("1 2"), JsonErrorKind::TrailingCharacters);
        assert!(parse("  {}  \n").is_ok());
        for bad in [
            "01",
            "1.",
            "1e",
            "1e+",
            "1.2.3",
            "-",
            "-Infinity",
            "1e999",
            "-1e999",
        ] {
            assert_eq!(kind(bad), JsonErrorKind::MalformedNumber, "{bad}");
        }
        for bad in ["+1", ".5", "NaN", "Infinity", "inf", "0x10z"] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert_eq!(kind("+1"), JsonErrorKind::UnexpectedChar);
        assert_eq!(kind("NaN"), JsonErrorKind::UnexpectedChar);
        assert_eq!(kind("\"a\tb\""), JsonErrorKind::BadEscape);
        assert_eq!(kind("\"\\ud83d\""), JsonErrorKind::BadEscape);
        assert_eq!(kind("\"\\ude00\""), JsonErrorKind::BadEscape);
        assert_eq!(kind("\"\\ud83dA\""), JsonErrorKind::BadEscape);
        assert_eq!(
            parse("\"\\ud83d\\ude00\"").unwrap(),
            JVal::Str("\u{1F600}".to_string())
        );
        assert_eq!(parse("1e-999").unwrap(), JVal::Num(0.0));
        for good in [
            "0",
            "-0",
            "1",
            "-1",
            "1.5",
            "1e5",
            "1E-7",
            "1e+21",
            "9007199254740992",
        ] {
            assert!(parse(good).is_ok(), "{good}");
        }
        assert_eq!(kind("9007199254740993"), JsonErrorKind::MalformedNumber);
    }

    fn breach(input: &str, limits: &JsonLimits) -> LimitBreach {
        match read(input, limits) {
            Err(JsonRefusal::LimitExceeded(b)) => b,
            other => panic!("expected a breach for {input:?}, got {other:?}"),
        }
    }

    #[test]
    fn a_bound_is_enforced_on_the_way_down_at_the_container_it_guards() {
        let depth2 = JsonLimits::depth_only(2);
        assert!(read("[[1]]", &depth2).is_ok());
        let b = breach("[[[1]]]", &depth2);
        assert_eq!(
            (b.limit, b.path),
            (JsonLimit::Depth, vec![PathSeg::Index(0), PathSeg::Index(0)])
        );

        let strings = JsonLimits {
            string_length: Some(3),
            ..JsonLimits::STRICT
        };
        assert!(read("\"\u{1F600}\u{1F600}\u{1F600}\"", &strings).is_ok());
        assert_eq!(breach("\"abcd\"", &strings).path, vec![]);
        assert_eq!(breach("{\"abcd\":1}", &strings).path, vec![]);
        assert_eq!(
            breach("{\"a\":[\"abcd\"]}", &strings).path,
            vec![PathSeg::Key("a".to_string()), PathSeg::Index(0)]
        );

        let arrays = JsonLimits {
            array_length: Some(2),
            ..JsonLimits::STRICT
        };
        assert_eq!(breach("[1,2,3", &arrays).limit, JsonLimit::ArrayLength);
        assert!(matches!(
            read("[1,%,3,4]", &arrays),
            Err(JsonRefusal::Malformed(_))
        ));

        let members = JsonLimits {
            object_members: Some(2),
            ..JsonLimits::STRICT
        };
        assert!(read("{\"a\":1,\"b\":2}", &members).is_ok());
        assert_eq!(
            breach("{\"a\":1,\"b\":2,\"c\":3}", &members).limit,
            JsonLimit::ObjectMembers
        );

        let bytes = JsonLimits {
            document_bytes: Some(4),
            ..JsonLimits::STRICT
        };
        assert_eq!(
            breach("\"\u{65E5}\"", &bytes).limit,
            JsonLimit::DocumentBytes
        );
    }

    /// The nesting stack spells a breach's path: every container around the
    /// breach, root first, each by the step to the child being read in it, and not
    /// the container a length bound or a member name is read in.
    #[test]
    fn a_breach_path_names_the_containers_around_it() {
        let key = |k: &str| PathSeg::Key(k.to_string());
        let at = |input: &str, limits: JsonLimits| {
            let b = breach(input, &limits);
            (b.limit, b.path)
        };
        let lim = |f: fn(&mut JsonLimits)| {
            let mut l = JsonLimits::STRICT;
            f(&mut l);
            l
        };
        assert_eq!(
            at("{\"a\":[[1]]}", JsonLimits::depth_only(2)),
            (JsonLimit::Depth, vec![key("a"), PathSeg::Index(0)])
        );
        assert_eq!(
            at("{\"a\":[1,2,3]}", lim(|l| l.array_length = Some(2))),
            (JsonLimit::ArrayLength, vec![key("a")])
        );
        assert_eq!(
            at("[0,{\"a\":1,\"b\":2}]", lim(|l| l.object_members = Some(1))),
            (JsonLimit::ObjectMembers, vec![PathSeg::Index(1)])
        );
        assert_eq!(
            at("[{\"x\":{\"abcd\":1}}]", lim(|l| l.string_length = Some(3))),
            (JsonLimit::StringLength, vec![PathSeg::Index(0), key("x")])
        );
        assert_eq!(
            at(
                "{\"a\":{\"b\":\"abcd\"}}",
                lim(|l| l.string_length = Some(3))
            ),
            (JsonLimit::StringLength, vec![key("a"), key("b")])
        );
    }

    #[test]
    fn control_characters_escape_in_lower_case_hex() {
        assert_eq!(escape_string("\u{001F}"), "\"\\u001f\"");
        assert_eq!(escape_string("\n"), "\"\\u000a\"");
        assert_eq!(escape_string("a/b"), "\"a/b\"");
        assert_eq!(escape_string("é✓"), "\"é✓\"");
    }
}
