//! The unit of measure: a unit as a value, its one canonical text, and the parser
//! that reads UCUM's case-sensitive syntax over the admitted subset. The twin of
//! the reference's `Unit` (`Fuaran.Core.Unit`), as far as a schema field needs
//! it: [`parse`], [`render`], the product algebra ([`mul`], [`div`], [`pow`]) and
//! [`compatible`]. The exact conversion factor between two compatible units is a
//! ratio over arbitrary-precision integers and is not ported here.
//!
//! A unit is a product of atoms, each under an optional SI prefix, with non-zero
//! integer exponents, held in ONE canonical form — factors ordered by symbol then
//! prefix, equal atoms merged, zero exponents dropped — so two units are equal
//! exactly when they are one product, and [`render`] writes each one text.
//!
//! **Positions** in a [`UnitRefusal`] are zero-based UTF-16 code-unit indices
//! into the text parsed, as the reference counts them.

use crate::prelude::*;

/// One factor of a unit: an atom (an admitted UCUM symbol, or a currency written
/// `[GBP]`) under an optional SI prefix (`""` for none). The derived order —
/// symbol, then prefix — is the canonical factor order (every symbol and prefix is
/// ASCII, so byte order is the reference's ordinal order).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct UnitAtom {
    pub symbol: String,
    pub prefix: String,
}

/// A unit as a value — twin of `UnitOfMeasure`. Built by [`parse`] and the
/// algebra; its factors are always in canonical form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitOfMeasure {
    factors: Vec<(UnitAtom, i32)>,
}

impl UnitOfMeasure {
    /// The factors in canonical order, each with its non-zero exponent.
    pub fn factors(&self) -> &[(UnitAtom, i32)] {
        &self.factors
    }
}

/// Why a text is not a unit — twin of `UnitRefusal`. Every case but `Empty`
/// names the offending token as written and its position.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnitRefusal {
    /// The text is empty.
    Empty,
    /// The text leaves the grammar at `position`: `found` is what stands there
    /// (empty at the end of the text), `expected` what the grammar wanted.
    Malformed {
        found: String,
        position: usize,
        expected: String,
    },
    /// A symbol the admitted vocabulary does not hold, with or without a prefix.
    UnknownAtom { token: String, position: usize },
    /// A UCUM annotation, `{...}`.
    Annotation { token: String, position: usize },
    /// A UCUM arbitrary unit (`[IU]`, `[CFU]`, …).
    ArbitraryUnit { token: String, position: usize },
    /// A special unit whose conversion is not a ratio (`Cel`, `[degF]`, `B`, …).
    NonRatioUnit { token: String, position: usize },
    /// An SI prefix on an atom that takes none (`kmin`, `k%`).
    PrefixNotAllowed { token: String, position: usize },
    /// A numeric factor other than `1` (`10*3`, `1000`).
    NumericFactor { token: String, position: usize },
}

impl UnitRefusal {
    /// The reference's sentence for the refusal (`ColumnCodec`'s `unitRefusalText`).
    pub fn text(&self) -> String {
        let at = |what: &str, token: &str, position: usize| {
            format!("{what} '{token}' at position {position}")
        };
        match self {
            UnitRefusal::Empty => "the text is empty".to_string(),
            UnitRefusal::Malformed {
                found,
                position,
                expected,
            } => format!("expected {expected} at position {position}, found '{found}'"),
            UnitRefusal::UnknownAtom { token, position } => {
                at("unknown unit symbol", token, *position)
            }
            UnitRefusal::Annotation { token, position } => at("annotation", token, *position),
            UnitRefusal::ArbitraryUnit { token, position } => {
                at("arbitrary unit", token, *position)
            }
            UnitRefusal::NonRatioUnit { token, position } => at("non-ratio unit", token, *position),
            UnitRefusal::PrefixNotAllowed { token, position } => {
                at("prefix not allowed on", token, *position)
            }
            UnitRefusal::NumericFactor { token, position } => {
                at("numeric factor", token, *position)
            }
        }
    }
}

// ─── The vocabulary (twin of `UnitVocabulary`) ───────────────────────────────

/// An admitted atom: whether SI prefixes apply, and its exponents over
/// `[length; mass; time; current; temperature; amount; luminous intensity]`.
/// (The reference also carries each atom's scale, which only the conversion
/// factor reads.)
struct AtomDef {
    symbol: &'static str,
    metric: bool,
    dims: [i8; 7],
}

const fn a(symbol: &'static str, metric: bool, dims: [i8; 7]) -> AtomDef {
    AtomDef {
        symbol,
        metric,
        dims,
    }
}

const NONE: [i8; 7] = [0; 7];

/// Every admitted atom by its canonical symbol.
const ATOMS: &[AtomDef] = &[
    a("m", true, [1, 0, 0, 0, 0, 0, 0]),
    a("g", true, [0, 1, 0, 0, 0, 0, 0]),
    a("s", true, [0, 0, 1, 0, 0, 0, 0]),
    a("A", true, [0, 0, 0, 1, 0, 0, 0]),
    a("K", true, [0, 0, 0, 0, 1, 0, 0]),
    a("mol", true, [0, 0, 0, 0, 0, 1, 0]),
    a("cd", true, [0, 0, 0, 0, 0, 0, 1]),
    a("rad", true, NONE),
    a("sr", true, NONE),
    a("Hz", true, [0, 0, -1, 0, 0, 0, 0]),
    a("N", true, [1, 1, -2, 0, 0, 0, 0]),
    a("Pa", true, [-1, 1, -2, 0, 0, 0, 0]),
    a("J", true, [2, 1, -2, 0, 0, 0, 0]),
    a("W", true, [2, 1, -3, 0, 0, 0, 0]),
    a("C", true, [0, 0, 1, 1, 0, 0, 0]),
    a("V", true, [2, 1, -3, -1, 0, 0, 0]),
    a("F", true, [-2, -1, 4, 2, 0, 0, 0]),
    a("Ohm", true, [2, 1, -3, -2, 0, 0, 0]),
    a("S", true, [-2, -1, 3, 2, 0, 0, 0]),
    a("Wb", true, [2, 1, -2, -1, 0, 0, 0]),
    a("T", true, [0, 1, -2, -1, 0, 0, 0]),
    a("H", true, [2, 1, -2, -2, 0, 0, 0]),
    a("lm", true, [0, 0, 0, 0, 0, 0, 1]),
    a("lx", true, [-2, 0, 0, 0, 0, 0, 1]),
    a("Bq", true, [0, 0, -1, 0, 0, 0, 0]),
    a("Gy", true, [2, 0, -2, 0, 0, 0, 0]),
    a("Sv", true, [2, 0, -2, 0, 0, 0, 0]),
    a("kat", true, [0, 0, -1, 0, 0, 1, 0]),
    a("L", true, [3, 0, 0, 0, 0, 0, 0]),
    a("t", true, [0, 1, 0, 0, 0, 0, 0]),
    a("bar", true, [-1, 1, -2, 0, 0, 0, 0]),
    a("min", false, [0, 0, 1, 0, 0, 0, 0]),
    a("h", false, [0, 0, 1, 0, 0, 0, 0]),
    a("d", false, [0, 0, 1, 0, 0, 0, 0]),
    a("wk", false, [0, 0, 1, 0, 0, 0, 0]),
    a("a", false, [0, 0, 1, 0, 0, 0, 0]),
    a("%", false, NONE),
    a("[ppm]", false, NONE),
    a("[ppb]", false, NONE),
    a("[in_i]", false, [1, 0, 0, 0, 0, 0, 0]),
    a("[ft_i]", false, [1, 0, 0, 0, 0, 0, 0]),
    a("[yd_i]", false, [1, 0, 0, 0, 0, 0, 0]),
    a("[mi_i]", false, [1, 0, 0, 0, 0, 0, 0]),
    a("[lb_av]", false, [0, 1, 0, 0, 0, 0, 0]),
    a("[oz_av]", false, [0, 1, 0, 0, 0, 0, 0]),
];

/// Spellings read as another atom: UCUM's lower-case litre.
const ALIASES: &[(&str, &str)] = &[("l", "L")];

/// UCUM's twenty prefixes and their powers of ten, `da` first so that `dam`
/// reads as decametre.
const PREFIXES: &[(&str, i32)] = &[
    ("da", 1),
    ("Y", 24),
    ("Z", 21),
    ("E", 18),
    ("P", 15),
    ("T", 12),
    ("G", 9),
    ("M", 6),
    ("k", 3),
    ("h", 2),
    ("d", -1),
    ("c", -2),
    ("m", -3),
    ("u", -6),
    ("n", -9),
    ("p", -12),
    ("f", -15),
    ("a", -18),
    ("z", -21),
    ("y", -24),
];

/// UCUM special units whose conversion is not a ratio.
const NON_RATIO: &[&str] = &[
    "Cel", "[degF]", "[degRe]", "[pH]", "Np", "B", "B[SPL]", "B[V]", "B[mV]", "B[uV]", "B[10.nV]",
    "B[W]", "B[kW]", "bit_s", "[p'diop]", "%[slope]", "[hp_X]", "[hp_C]", "[hp_M]", "[hp_Q]",
    "[kp_X]", "[kp_C]", "[kp_M]", "[kp_Q]",
];

/// UCUM's arbitrary units, read before the currency rule.
const ARBITRARY: &[&str] = &[
    "[iU]",
    "[IU]",
    "[arb'U]",
    "[USP'U]",
    "[GPL'U]",
    "[MPL'U]",
    "[APL'U]",
    "[beth'U]",
    "[anti'Xa'U]",
    "[todd'U]",
    "[dye'U]",
    "[smgy'U]",
    "[bdsk'U]",
    "[ka'U]",
    "[knk'U]",
    "[mclg'U]",
    "[tb'U]",
    "[CCID_50]",
    "[TCID_50]",
    "[EID_50]",
    "[PFU]",
    "[FFU]",
    "[CFU]",
    "[IR]",
    "[BAU]",
    "[AU]",
    "[Amb'a'1'U]",
    "[PNU]",
    "[Lf]",
    "[D'ag'U]",
    "[FEU]",
    "[ELU]",
    "[EU]",
];

fn atom_def(symbol: &str) -> Option<&'static AtomDef> {
    ATOMS.iter().find(|d| d.symbol == symbol)
}

/// An ISO 4217 alphabetic code in brackets — `[` three ASCII capitals `]` — and
/// not one of UCUM's arbitrary units. Twin of `UnitVocabulary.isCurrency`.
pub fn is_currency(tok: &str) -> bool {
    let b = tok.as_bytes();
    b.len() == 5
        && b[0] == b'['
        && b[4] == b']'
        && b[1..4].iter().all(u8::is_ascii_uppercase)
        && !ARBITRARY.contains(&tok)
}

/// The canonical symbol of an admitted atom, through the aliases.
fn try_atom(tok: &str) -> Option<&'static str> {
    let canonical = ALIASES
        .iter()
        .find(|(k, _)| *k == tok)
        .map_or(tok, |(_, v)| *v);
    atom_def(canonical).map(|d| d.symbol)
}

// ─── The algebra ─────────────────────────────────────────────────────────────

const MAX_EXP: i64 = i32::MAX as i64;

/// Merge two factor lists, the second scaled by `sign`; `None` where an exponent
/// leaves `±2147483647`.
fn try_combine(
    a: &[(UnitAtom, i32)],
    sign: i64,
    b: &[(UnitAtom, i32)],
) -> Option<Vec<(UnitAtom, i32)>> {
    let mut m: BTreeMap<UnitAtom, i64> =
        a.iter().map(|(k, e)| (k.clone(), i64::from(*e))).collect();
    for (atom, e) in b {
        let sum = m.get(atom).copied().unwrap_or(0) + sign * i64::from(*e);
        if !(-MAX_EXP..=MAX_EXP).contains(&sum) {
            return None;
        }
        m.insert(atom.clone(), sum);
    }
    Some(
        m.into_iter()
            .filter(|(_, e)| *e != 0)
            .map(|(k, e)| (k, e as i32))
            .collect(),
    )
}

/// The dimensionless unit, `1` — twin of `Unit.dimensionless`.
pub fn dimensionless() -> UnitOfMeasure {
    UnitOfMeasure {
        factors: Vec::new(),
    }
}

/// The product of two units — twin of `Unit.mul`; `None` where an exponent would
/// leave `±2147483647` (the reference raises there).
pub fn mul(a: &UnitOfMeasure, b: &UnitOfMeasure) -> Option<UnitOfMeasure> {
    try_combine(&a.factors, 1, &b.factors).map(|factors| UnitOfMeasure { factors })
}

/// The quotient `a / b` — twin of `Unit.div`; `None` where an exponent would
/// leave `±2147483647`.
pub fn div(a: &UnitOfMeasure, b: &UnitOfMeasure) -> Option<UnitOfMeasure> {
    try_combine(&a.factors, -1, &b.factors).map(|factors| UnitOfMeasure { factors })
}

/// `u` to the integer power `n` (`pow u 0` is dimensionless) — twin of
/// `Unit.pow`; `None` where an exponent would leave `±2147483647`.
pub fn pow(u: &UnitOfMeasure, n: i32) -> Option<UnitOfMeasure> {
    if n == 0 {
        return Some(dimensionless());
    }
    let mut factors = Vec::with_capacity(u.factors.len());
    for (atom, e) in &u.factors {
        let p = i64::from(*e) * i64::from(n);
        if !(-MAX_EXP..=MAX_EXP).contains(&p) {
            return None;
        }
        factors.push((atom.clone(), p as i32));
    }
    Some(UnitOfMeasure { factors })
}

/// A unit's dimension key: a base dimension's index, or a currency.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum DimKey {
    Base(usize),
    Currency(String),
}

fn dimension(u: &UnitOfMeasure) -> BTreeMap<DimKey, i64> {
    let mut m: BTreeMap<DimKey, i64> = BTreeMap::new();
    for (atom, e) in &u.factors {
        let e = i64::from(*e);
        if is_currency(&atom.symbol) {
            *m.entry(DimKey::Currency(atom.symbol.clone())).or_insert(0) += e;
        } else if let Some(def) = atom_def(&atom.symbol) {
            for (i, x) in def.dims.iter().enumerate() {
                *m.entry(DimKey::Base(i)).or_insert(0) += i64::from(*x) * e;
            }
        }
    }
    m.retain(|_, x| *x != 0);
    m
}

/// Whether two units measure one dimension — twin of `Unit.compatible`. Every
/// currency is a dimension of its own.
pub fn compatible(a: &UnitOfMeasure, b: &UnitOfMeasure) -> bool {
    dimension(a) == dimension(b)
}

/// The unit's one canonical text — twin of `Unit.render`: the factors with a
/// positive exponent joined by `.`, then each negative one as `/` and the factor,
/// an exponent written as its magnitude after the atom and omitted where it is 1;
/// `1` for the dimensionless unit. `km/h`, `kg.m/s2`, `/min`, `k[GBP]`.
pub fn render(u: &UnitOfMeasure) -> String {
    let atom_text = |(a, e): &(UnitAtom, i32)| {
        let e = e.unsigned_abs();
        if e == 1 {
            format!("{}{}", a.prefix, a.symbol)
        } else {
            format!("{}{}{e}", a.prefix, a.symbol)
        }
    };
    let num: Vec<String> = u
        .factors
        .iter()
        .filter(|(_, e)| *e > 0)
        .map(atom_text)
        .collect();
    let den: Vec<String> = u
        .factors
        .iter()
        .filter(|(_, e)| *e < 0)
        .map(atom_text)
        .collect();
    if num.is_empty() && den.is_empty() {
        return "1".to_string();
    }
    let mut out = num.join(".");
    for d in den {
        out.push('/');
        out.push_str(&d);
    }
    out
}

// ─── The parser ──────────────────────────────────────────────────────────────

fn is_letter(c: u16) -> bool {
    (u16::from(b'a')..=u16::from(b'z')).contains(&c)
        || (u16::from(b'A')..=u16::from(b'Z')).contains(&c)
}

fn is_digit(c: u16) -> bool {
    (u16::from(b'0')..=u16::from(b'9')).contains(&c)
}

fn is(c: u16, ch: u8) -> bool {
    c == u16::from(ch)
}

fn sub(text: &[u16], from: usize, to: usize) -> String {
    String::from_utf16_lossy(&text[from..to])
}

fn index_of(text: &[u16], ch: u8, from: usize) -> Option<usize> {
    text[from..]
        .iter()
        .position(|c| is(*c, ch))
        .map(|i| i + from)
}

fn atom_of(symbol: &str, prefix: &str) -> UnitAtom {
    UnitAtom {
        symbol: symbol.to_string(),
        prefix: prefix.to_string(),
    }
}

/// Resolve one symbol token to an atom, or to the refusal naming its class.
fn resolve(tok: &str, position: usize) -> Result<UnitAtom, UnitRefusal> {
    if let Some(sym) = try_atom(tok) {
        return Ok(atom_of(sym, ""));
    }
    let token = || tok.to_string();
    if NON_RATIO.contains(&tok) {
        return Err(UnitRefusal::NonRatioUnit {
            token: token(),
            position,
        });
    }
    if ARBITRARY.contains(&tok) {
        return Err(UnitRefusal::ArbitraryUnit {
            token: token(),
            position,
        });
    }
    if is_currency(tok) {
        return Ok(atom_of(tok, ""));
    }
    for (p, _) in PREFIXES {
        // The reference's `startsWith` is strict: the prefix alone is not a match.
        if tok.len() > p.len() && tok.starts_with(p) {
            let rest = &tok[p.len()..];
            match try_atom(rest) {
                Some(sym) if atom_def(sym).is_some_and(|d| d.metric) => {
                    return Ok(atom_of(sym, p));
                }
                Some(_) => {
                    return Err(UnitRefusal::PrefixNotAllowed {
                        token: token(),
                        position,
                    });
                }
                None => {
                    if NON_RATIO.contains(&rest) {
                        return Err(UnitRefusal::NonRatioUnit {
                            token: token(),
                            position,
                        });
                    }
                    if ARBITRARY.contains(&rest) {
                        return Err(UnitRefusal::ArbitraryUnit {
                            token: token(),
                            position,
                        });
                    }
                    if is_currency(rest) {
                        return Ok(atom_of(rest, p));
                    }
                }
            }
        }
    }
    Err(UnitRefusal::UnknownAtom {
        token: token(),
        position,
    })
}

/// One open term of the parse: its product so far, the sign the next component
/// takes, the sign the whole term takes in its enclosing term, and where it opened.
struct Frame {
    acc: Vec<(UnitAtom, i32)>,
    next: i64,
    outer: i64,
    opened: usize,
}

const EXP_RANGE: &str = "an exponent within ±2147483647";

fn malformed(found: String, position: usize, expected: &str) -> UnitRefusal {
    UnitRefusal::Malformed {
        found,
        position,
        expected: expected.to_string(),
    }
}

/// Read a unit from its text — twin of `Unit.parse`: atoms under an optional
/// prefix, each with an optional signed integer exponent; products `.`,
/// quotients `/` (left-associative, with an optional leading `/`), parentheses,
/// and the factor `1`. No white space is read. Iterative over an explicit stack of
/// open terms, so nesting depth never reaches the machine stack.
pub fn parse(text: &str) -> Result<UnitOfMeasure, UnitRefusal> {
    let t: Vec<u16> = text.encode_utf16().collect();
    let n = t.len();
    if n == 0 {
        return Err(UnitRefusal::Empty);
    }
    let char_at = |i: usize| {
        if i < n {
            sub(&t, i, i + 1)
        } else {
            String::new()
        }
    };
    let mut stack = vec![Frame {
        acc: Vec::new(),
        next: 1,
        outer: 1,
        opened: 0,
    }];
    let mut pos = 0usize;
    let mut expect_component = true;
    let mut term_start = true;

    loop {
        if expect_component {
            if pos >= n {
                return Err(malformed(String::new(), pos, "a unit"));
            }
            let c = t[pos];
            if term_start && is(c, b'/') {
                if let Some(top) = stack.last_mut() {
                    top.next = -1;
                }
                pos += 1;
                term_start = false;
            } else if is(c, b'(') {
                let outer = stack.last().map_or(1, |f| f.next);
                stack.push(Frame {
                    acc: Vec::new(),
                    next: 1,
                    outer,
                    opened: pos,
                });
                pos += 1;
                term_start = true;
            } else if is_digit(c) {
                let start = pos;
                while pos < n && is_digit(t[pos]) {
                    pos += 1;
                }
                if pos < n && (is(t[pos], b'*') || is(t[pos], b'^')) {
                    pos += 1;
                    if pos < n && (is(t[pos], b'+') || is(t[pos], b'-')) {
                        pos += 1;
                    }
                    while pos < n && is_digit(t[pos]) {
                        pos += 1;
                    }
                }
                let tok = sub(&t, start, pos);
                if tok == "1" {
                    expect_component = false;
                    term_start = false;
                } else {
                    return Err(UnitRefusal::NumericFactor {
                        token: tok,
                        position: start,
                    });
                }
            } else if is(c, b'{') {
                return Err(annotation(&t, pos));
            } else if is_letter(c) || is(c, b'[') || is(c, b'%') {
                let start = pos;
                while pos < n
                    && (is_letter(t[pos])
                        || is(t[pos], b'_')
                        || is(t[pos], b'\'')
                        || is(t[pos], b'%')
                        || is(t[pos], b'['))
                {
                    if is(t[pos], b'[') {
                        match index_of(&t, b']', pos) {
                            None => {
                                return Err(malformed(
                                    sub(&t, pos, n),
                                    pos,
                                    "']' closing the symbol",
                                ));
                            }
                            Some(close) => pos = close + 1,
                        }
                    } else {
                        pos += 1;
                    }
                }
                let tok = sub(&t, start, pos);
                let atom = resolve(&tok, start)?;
                let exp_start = pos;
                let mut sign = 1i64;
                if pos < n && (is(t[pos], b'+') || is(t[pos], b'-')) {
                    if is(t[pos], b'-') {
                        sign = -1;
                    }
                    pos += 1;
                }
                let digits_start = pos;
                let mut value = 0i64;
                let mut over = false;
                while pos < n && is_digit(t[pos]) {
                    if !over {
                        value = value * 10 + i64::from(t[pos] - u16::from(b'0'));
                        if value > MAX_EXP {
                            over = true;
                        }
                    }
                    pos += 1;
                }
                if pos == digits_start && pos > exp_start {
                    return Err(malformed(char_at(pos), pos, "the digits of an exponent"));
                }
                if over {
                    return Err(malformed(sub(&t, exp_start, pos), exp_start, EXP_RANGE));
                }
                let e = if pos == exp_start { 1 } else { sign * value };
                let comp: Vec<(UnitAtom, i32)> = if e == 0 {
                    Vec::new()
                } else {
                    vec![(atom, e as i32)]
                };
                if let Some(top) = stack.last_mut() {
                    match try_combine(&top.acc, top.next, &comp) {
                        Some(acc) => top.acc = acc,
                        None => return Err(malformed(sub(&t, start, pos), start, EXP_RANGE)),
                    }
                }
                expect_component = false;
                term_start = false;
            } else {
                return Err(malformed(char_at(pos), pos, "a unit"));
            }
        } else if pos >= n {
            return match stack.len() {
                1 => Ok(UnitOfMeasure {
                    factors: stack.pop().map(|f| f.acc).unwrap_or_default(),
                }),
                0 => Err(malformed(String::new(), pos, "a unit")),
                _ => {
                    let opened = stack.last().map_or(0, |f| f.opened);
                    Err(malformed(
                        String::new(),
                        pos,
                        &format!("')' closing the '(' at {opened}"),
                    ))
                }
            };
        } else {
            let c = t[pos];
            if is(c, b'.') || is(c, b'/') {
                if let Some(top) = stack.last_mut() {
                    top.next = if is(c, b'.') { 1 } else { -1 };
                }
                pos += 1;
                expect_component = true;
            } else if is(c, b')') {
                if stack.len() < 2 {
                    return Err(malformed(")".to_string(), pos, "'.', '/' or the end"));
                }
                let at = pos;
                pos += 1;
                let inner = stack.pop().expect("two frames are open");
                let outer = stack.last_mut().expect("two frames are open");
                // The closed term is one component of the term that encloses it.
                match try_combine(&outer.acc, inner.outer, &inner.acc) {
                    Some(acc) => outer.acc = acc,
                    None => return Err(malformed(")".to_string(), at, EXP_RANGE)),
                }
            } else if is(c, b'{') {
                return Err(annotation(&t, pos));
            } else {
                let expected = if stack.len() > 1 {
                    "'.', '/' or ')'"
                } else {
                    "'.', '/' or the end"
                };
                return Err(malformed(char_at(pos), pos, expected));
            }
        }
    }
}

/// The refusal of an annotation opening at `pos`: the annotation itself, or the
/// unclosed text.
fn annotation(t: &[u16], pos: usize) -> UnitRefusal {
    match index_of(t, b'}', pos) {
        None => malformed(sub(t, pos, t.len()), pos, "'}' closing the annotation"),
        Some(close) => UnitRefusal::Annotation {
            token: sub(t, pos, close + 1),
            position: pos,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canon(text: &str) -> String {
        render(&parse(text).unwrap())
    }

    #[test]
    fn the_canonical_text_is_one_per_unit() {
        assert_eq!(canon("s-1.m"), "m/s");
        assert_eq!(canon("m/s/s"), "m/s2");
        assert_eq!(canon("l"), "L");
        assert_eq!(canon("1"), "1");
        assert_eq!(canon("/min"), "/min");
        assert_eq!(canon("kg.m/s2"), "kg.m/s2");
        assert_eq!(canon("m/(s.s)"), "m/s2");
        assert_eq!(canon("c[GBP]"), "c[GBP]");
        assert_eq!(canon("m.m-1"), "1");
        assert!(compatible(&parse("km/h").unwrap(), &parse("m/s").unwrap()));
        assert!(!compatible(
            &parse("[GBP]").unwrap(),
            &parse("[USD]").unwrap()
        ));
    }

    #[test]
    fn every_refusal_names_its_class() {
        assert_eq!(parse(""), Err(UnitRefusal::Empty));
        assert!(matches!(
            parse("furlong"),
            Err(UnitRefusal::UnknownAtom { .. })
        ));
        assert!(matches!(
            parse("Cel"),
            Err(UnitRefusal::NonRatioUnit { .. })
        ));
        assert!(matches!(
            parse("[IU]"),
            Err(UnitRefusal::ArbitraryUnit { .. })
        ));
        assert!(matches!(
            parse("kmin"),
            Err(UnitRefusal::PrefixNotAllowed { .. })
        ));
        assert!(matches!(
            parse("10*3"),
            Err(UnitRefusal::NumericFactor { .. })
        ));
        assert!(matches!(
            parse("m{tall}"),
            Err(UnitRefusal::Annotation { position: 1, .. })
        ));
        assert!(matches!(parse("(m"), Err(UnitRefusal::Malformed { .. })));
        assert!(matches!(
            parse("m3000000000"),
            Err(UnitRefusal::Malformed { .. })
        ));
    }
}
