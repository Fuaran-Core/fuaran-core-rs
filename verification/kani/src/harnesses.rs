//! The harnesses. What closes on a 16 GB machine is the writer's escape over the
//! empty and every one-byte ASCII string; everything past that — the longer escape
//! rungs and every reader rung — is behind the `frontier` feature because none of
//! it closes there. The measurements are in `../README.md`, and the harnesses stay
//! so the next attempt is one flag away.

use fuaran_core::json::escape_string;

// ─── The canonical escape ────────────────────────────────────────────────────

/// Rule 6, byte by byte: what each ASCII byte's escape is long.
fn escaped_len(b: u8) -> usize {
    match b {
        b'"' | b'\\' => 2,
        b if b < 0x20 => 6,
        _ => 1,
    }
}

/// `escape_string` over every ASCII string of exactly `M` bytes (symbolic bytes,
/// concrete length): it returns, the result is quoted, and it is exactly as long
/// as rule 6 says — `"` and `\` two bytes, a control character six, anything else
/// one. ASCII because the escape never touches a byte above 0x7F, and a symbolic
/// multi-byte sequence would spend the checker's budget on UTF-8 validation. The
/// length is concrete per rung because a symbolic length makes the output's
/// capacity symbolic, and an allocation of symbolic size is what exhausts the
/// checker (see `../README.md`).
fn escape_within_rule_6<const M: usize>() {
    let buf: [u8; M] = kani::any();
    for b in buf.iter() {
        kani::assume(*b < 0x80);
    }
    let text = unsafe { core::str::from_utf8_unchecked(&buf) };
    let out = escape_string(text);
    assert!(out.starts_with('"') && out.ends_with('"'));
    let expected: usize = buf.iter().map(|b| escaped_len(*b)).sum();
    assert!(out.len() == expected + 2);
}

macro_rules! escape_rung {
    ($($name:ident = $n:literal unwind $unwind:literal;)*) => {$(
        #[kani::proof]
        #[kani::unwind($unwind)]
        fn $name() {
            escape_within_rule_6::<$n>();
        }
    )*};
}

escape_rung! {
    escape_len0 = 0 unwind 2;
    escape_len1 = 1 unwind 3;
}

/// Two more symbolic bytes through the writer exhaust 15 GB (`../README.md`), so
/// these rungs are frontier, not gate.
#[cfg(feature = "frontier")]
escape_rung! {
    escape_len2 = 2 unwind 4;
    escape_len3 = 3 unwind 5;
}

// ─── The limited reader (feature `frontier`; does not close on 16 GB) ────────

#[cfg(feature = "frontier")]
mod reader {
    extern crate alloc;

    use core::sync::atomic::{AtomicUsize, Ordering};
    use std::alloc::{GlobalAlloc, Layout, System};

    use fuaran_core::json::kani_support::{Shape, read_shape};
    use fuaran_core::json::{JsonLimit, JsonLimits, JsonRefusal, read};

    /// The peak live heap, in bytes, no read the harnesses cover may pass, under
    /// any limits record. A ceiling to be found by lowering it until a rung fails;
    /// no rung has closed, so it is a bound to be measured, not a proved one.
    const PEAK_CEILING: usize = 4096;

    /// Counts the live heap of the current harness and its peak.
    struct Counting;

    static LIVE: AtomicUsize = AtomicUsize::new(0);
    static PEAK: AtomicUsize = AtomicUsize::new(0);

    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
            let live = LIVE.load(Ordering::Relaxed) + layout.size();
            LIVE.store(live, Ordering::Relaxed);
            if live > PEAK.load(Ordering::Relaxed) {
                PEAK.store(live, Ordering::Relaxed);
            }
            unsafe { System.alloc(layout) }
        }
        unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
            LIVE.store(
                LIVE.load(Ordering::Relaxed) - layout.size(),
                Ordering::Relaxed,
            );
            unsafe { System.dealloc(ptr, layout) }
        }
    }

    #[global_allocator]
    static ALLOC: Counting = Counting;

    /// Every valid UTF-8 text of at most `M` bytes: symbolic bytes, symbolic length.
    fn any_text<const M: usize>(buf: &[u8; M]) -> Option<&str> {
        let len: usize = kani::any();
        kani::assume(len <= M);
        core::str::from_utf8(&buf[..len]).ok()
    }

    /// Absent, or any value in 0..=M+1: past M a bound cannot bite on an M-byte
    /// input, so the range covers every behaviour of every bound.
    fn any_bound<const M: usize>() -> Option<usize> {
        let on: bool = kani::any();
        let v: usize = kani::any();
        kani::assume(v <= M + 1);
        if on { Some(v) } else { None }
    }

    /// A limits record with the given (concrete) depth and every other bound
    /// symbolic. The depth is concrete because it is what bounds the reader's
    /// recursion: a rung is instantiated once per depth in 0..=N, and a depth past
    /// N behaves exactly as N does on an N-byte input (each level costs at least
    /// its opening byte), so the instances together cover every depth, the
    /// reader's own 512 among them.
    fn limits<const M: usize>(depth: usize) -> JsonLimits {
        JsonLimits {
            depth,
            string_length: any_bound::<M>(),
            array_length: any_bound::<M>(),
            object_members: any_bound::<M>(),
            document_bytes: any_bound::<M>(),
        }
    }

    /// Stands in for `<f64 as FromStr>::from_str`, core's decimal-to-float reader,
    /// whose bignum slow path no bounded model checker unwinds in useful time. The
    /// reader calls it only on a token that has passed its own RFC 8259 grammar
    /// check, and core's reader accepts every such token, so `Ok(any f64)`
    /// over-approximates it: every double, the non-finite ones included, is a
    /// possible answer, and the reader's handling of each is what is proved.
    fn any_f64(_token: &str) -> Result<f64, core::num::ParseFloatError> {
        Ok(kani::any())
    }

    /// Stands in for `fuaran_core::number::format_number`. The reader calls it
    /// only on an integer token of sixteen digits or more, which no input the
    /// rungs cover holds, so the stub asserts it is never reached.
    fn unreachable_format(_n: f64) -> String {
        kani::assert(false, "format_number is unreachable below 16 bytes");
        String::new()
    }

    /// Stands in for `alloc::fmt::format`, the body of `format!`. The reader
    /// formats only the human-readable `message` of a refusal; the kind, offset,
    /// limit and path are built without it. Core's formatting machinery is not
    /// this crate's code, so the message is modelled as empty.
    fn empty_message(_args: core::fmt::Arguments<'_>) -> String {
        String::new()
    }

    /// The value a bound names, or `usize::MAX` when it is absent.
    fn cap(b: Option<usize>) -> usize {
        b.unwrap_or(usize::MAX)
    }

    /// The reader over every text of at most `M` bytes, at limits depth `depth`:
    ///
    /// 1. it returns — no panic, overflow, out-of-bounds index or undefined behaviour;
    /// 2. an accepted value is within every bound it was read under;
    /// 3. a breach names the bound it passed, with that bound's declared value;
    /// 4. a document over `documentBytes` is refused before a byte of heap is taken;
    /// 5. the peak live heap of the read is at most `PEAK_CEILING`.
    fn reader_within_its_limits<const M: usize>(depth: usize) {
        let buf: [u8; M] = kani::any();
        let Some(text) = any_text(&buf) else { return };
        let lim = limits::<M>(depth);
        let before = LIVE.load(Ordering::Relaxed);
        PEAK.store(before, Ordering::Relaxed);
        let result = read_shape(text, &lim);
        let peak = PEAK.load(Ordering::Relaxed) - before;
        match &result {
            Ok(Shape {
                depth: d,
                string_length,
                array_length,
                object_members,
            }) => {
                assert!(*d <= lim.depth);
                assert!(*string_length <= cap(lim.string_length));
                assert!(*array_length <= cap(lim.array_length));
                assert!(*object_members <= cap(lim.object_members));
            }
            Err(JsonRefusal::LimitExceeded(b)) => {
                let declared = match b.limit {
                    JsonLimit::Depth => Some(lim.depth),
                    JsonLimit::StringLength => lim.string_length,
                    JsonLimit::ArrayLength => lim.array_length,
                    JsonLimit::ObjectMembers => lim.object_members,
                    JsonLimit::DocumentBytes => lim.document_bytes,
                };
                assert!(declared == Some(b.value));
            }
            Err(JsonRefusal::Malformed(_)) => {}
        }
        if text.len() > cap(lim.document_bytes) {
            assert!(matches!(
                &result,
                Err(JsonRefusal::LimitExceeded(b)) if b.limit == JsonLimit::DocumentBytes
            ));
            assert!(peak == 0);
        }
        assert!(peak <= PEAK_CEILING);
    }

    /// The public entry, `read`, building the real value model: it returns on
    /// every text of at most `M` bytes at limits depth `depth`.
    fn public_read_returns<const M: usize>(depth: usize) {
        let buf: [u8; M] = kani::any();
        let Some(text) = any_text(&buf) else { return };
        let _ = read(text, &limits::<M>(depth));
    }

    /// One proof per rung: `$name` runs `$body::<$n>($depth)` with the loops
    /// unwound `$unwind` times — the input length plus two, which the reader's
    /// whitespace and token loops need over an input of that length — and the
    /// three stubs above in place.
    macro_rules! rung {
        ($($name:ident = $body:ident :: <$n:literal> ($depth:literal) unwind $unwind:literal;)*) => {$(
            #[kani::proof]
            #[kani::unwind($unwind)]
            #[kani::stub(<f64 as core::str::FromStr>::from_str, any_f64)]
            #[kani::stub(fuaran_core::number::format_number, unreachable_format)]
            #[kani::stub(alloc::fmt::format, empty_message)]
            fn $name() {
                $body::<$n>($depth);
            }
        )*};
    }

    rung! {
        limits_n1_d0 = reader_within_its_limits::<1>(0) unwind 3;
        limits_n2_d0 = reader_within_its_limits::<2>(0) unwind 4;
        limits_n2_d1 = reader_within_its_limits::<2>(1) unwind 4;
        limits_n2_d2 = reader_within_its_limits::<2>(2) unwind 4;
        read_n1_d0 = public_read_returns::<1>(0) unwind 3;
        read_n2_d0 = public_read_returns::<2>(0) unwind 4;
    }
}
