//! The capability decision path allocates nothing once the registry is built
//! (DECISIONS.md D12): every row of the table D11.7 measured, asserted at zero,
//! with a counting global allocator around exactly the call under test.
//!
//! A global allocator is process-wide, so this leg is its own test binary and
//! counts on the measuring thread only: the harness's own threads allocate
//! while a test runs, and a process-wide count would read them. The window is
//! armed after the registry, the arguments and the grant are built, and
//! disarmed before the call's answer is dropped. The first row is a known
//! allocation, so the count is shown to count before any row is read as zero.
//!
//! `run.ps1` runs it on the embedded profile (the library built without `std`)
//! and `cargo test --all-features` on the hosted one; it runs on the build host,
//! because no emulator is in the gate.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;

use fuaran_core::capability::{
    Capability, CapabilityRegistry, InvokeError, PolicyDecision, PolicyGate,
};

// ─── The counting allocator ──────────────────────────────────────────────────

struct Counting;

std::thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<usize> = const { Cell::new(0) };
    static BYTES: Cell<usize> = const { Cell::new(0) };
}

/// Count one allocation of `size` bytes, when this thread's window is armed.
fn count(size: usize) {
    if ARMED.try_with(Cell::get).unwrap_or(false) {
        let _ = ALLOCS.try_with(|a| a.set(a.get() + 1));
        let _ = BYTES.try_with(|b| b.set(b.get() + size));
    }
}

// SAFETY: every call is forwarded to the system allocator unchanged; the
// counters are const-initialised thread-locals with no destructor, so reading
// them never allocates and never re-enters the allocator.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        count(l.size());
        unsafe { System.alloc(l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        count(l.size());
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        count(n);
        unsafe { System.realloc(p, l, n) }
    }
}

#[global_allocator]
static COUNTING: Counting = Counting;

/// One row: the allocations and bytes `f` asks for, on this thread, and its
/// answer (dropped by the caller, outside the window).
fn measure<T>(f: impl FnOnce() -> T) -> ((usize, usize), T) {
    ALLOCS.with(|a| a.set(0));
    BYTES.with(|b| b.set(0));
    ARMED.with(|a| a.set(true));
    let out = black_box(f());
    ARMED.with(|a| a.set(false));
    ((ALLOCS.with(Cell::get), BYTES.with(Cell::get)), out)
}

// ─── The registry under test: the gate family's, as an embedded host builds it ─

const WRITE: &str = r#"{"$type":"capability","determinism":"deterministic","id":"write","placement":{"$type":"server"},"result":{"$type":"nothing"},"signature":{"effect":{"determinism":"deterministic","host":"pure"},"holes":[{"addr":"n","kind":"value","name":"n","required":true,"space":{"$type":"intRange","max":99,"min":0}},{"addr":"x","kind":"value","name":"x","required":true,"space":{"$type":"floatRange","max":10,"min":0}},{"addr":"s","kind":"value","name":"s","required":true,"space":{"$type":"anyString"}}],"name":"write"}}"#;
const READ: &str = r#"{"$type":"capability","determinism":"deterministic","id":"read","placement":{"$type":"server"},"result":{"$type":"intRange","max":99,"min":0},"signature":{"effect":{"determinism":"deterministic","host":"pure"},"holes":[{"addr":"n","kind":"value","name":"n","required":true,"space":{"$type":"intRange","max":99,"min":0}}],"name":"read"}}"#;
const ERASE: &str = r#"{"$type":"capability","determinism":"deterministic","id":"erase","placement":{"$type":"server"},"result":{"$type":"nothing"},"signature":{"effect":{"determinism":"deterministic","host":"pure"},"holes":[{"addr":"n","kind":"value","name":"n","required":true,"space":{"$type":"intRange","max":99,"min":0}}],"name":"erase"}}"#;

/// The context an embedded host decides on: an integer.
const TRUSTED: u8 = 0;
const UNTRUSTED: u8 = 1;

/// The family's `ceiling` gate over a `u8` context, its denial's guidance built
/// once with the gate and carried by every denial it makes.
fn ceiling() -> PolicyGate<u8> {
    let message: String = "erasing is not open to this context".into();
    let alternatives: Vec<String> = vec!["read".into()];
    PolicyGate::new(
        "ceiling",
        move |ctx: &u8, c: &Capability, _: &[(String, String)]| match (*ctx, c.id.as_str()) {
            (UNTRUSTED, "write") => PolicyDecision::NeedsApproval,
            (UNTRUSTED, "erase") => {
                PolicyDecision::deny_with(message.clone(), alternatives.clone())
            }
            _ => PolicyDecision::Allow,
        },
    )
}

fn registry() -> CapabilityRegistry<u8> {
    [WRITE, READ, ERASE]
        .iter()
        .fold(CapabilityRegistry::empty(), |r, text| {
            r.register(Capability::decode(text).expect("a declaration"))
                .expect("admitted")
        })
        .with_gate(ceiling())
}

fn bindings(xs: &[(&str, &str)]) -> Vec<(String, String)> {
    xs.iter()
        .map(|(a, v)| (a.to_string(), v.to_string()))
        .collect()
}

#[test]
fn the_decision_path_allocates_nothing_once_the_registry_is_built() {
    let r = registry();
    let write = r.try_find("write").expect("write").clone();
    let read = r.try_find("read").expect("read").clone();
    let w_args = bindings(&[("n", "7"), ("x", "1.5"), ("s", "a")]);
    let r_args = bindings(&[("n", "3")]);
    let e_args = bindings(&[("n", "3")]);
    let own = write.grant(&w_args);

    let mut table: Vec<(&str, (usize, usize))> = Vec::new();

    // The calibration row: a known allocation of 16 bytes, counted once.
    let (known, v) = measure(|| Vec::<u8>::with_capacity(16));
    drop(v);
    assert_eq!(
        known,
        (1, 16),
        "the counting allocator counts a known allocation"
    );

    let (n, d) = measure(|| r.decide_named(&TRUSTED, None, "read", &r_args));
    assert_eq!(d, Ok(("".to_string(), PolicyDecision::Allow)));
    table.push(("decide_named, allowed, one binding", n));

    let (n, d) = measure(|| r.decide_named(&UNTRUSTED, None, "write", &w_args));
    assert_eq!(
        d,
        Ok(("ceiling".to_string(), PolicyDecision::NeedsApproval))
    );
    table.push(("decide_named, needs approval, no grant, three bindings", n));

    let (n, d) = measure(|| r.decide_named(&UNTRUSTED, Some(&own), "write", &w_args));
    assert_eq!(d, Ok(("ceiling".to_string(), PolicyDecision::Allow)));
    table.push((
        "decide_named, needs approval, its own grant, three bindings",
        n,
    ));

    let (n, d) = measure(|| r.decide_named(&UNTRUSTED, None, "erase", &e_args));
    assert!(matches!(d, Ok((ref p, PolicyDecision::Deny { .. })) if p == "ceiling"));
    drop(d);
    table.push(("decide_named, denied, one binding", n));

    let (n, d) = measure(|| r.dispatch(&TRUSTED, None, "read", &r_args, |_| Ok(())));
    assert_eq!(d, Ok(()));
    table.push(("dispatch, allowed, one binding", n));

    let (n, d) = measure(|| r.dispatch(&UNTRUSTED, Some(&own), "write", &w_args, |_| Ok(())));
    assert_eq!(d, Ok(()));
    table.push(("dispatch, granted, three bindings", n));

    let (n, d) = measure(|| r.dispatch(&UNTRUSTED, None, "write", &w_args, |_| Ok(())));
    assert!(matches!(d, Err(InvokeError::ApprovalRequired { .. })));
    drop(d);
    table.push(("dispatch, parked (approval required), three bindings", n));

    let (n, d) = measure(|| r.dispatch(&UNTRUSTED, None, "erase", &e_args, |_| Ok(())));
    assert!(matches!(d, Err(InvokeError::PolicyRefused { .. })));
    drop(d);
    table.push(("dispatch, denied, one binding", n));

    let (n, d) = measure(|| read.invocation_identity(&r_args));
    drop(d);
    table.push(("invocation_identity, one binding", n));

    let (n, d) = measure(|| write.invocation_identity(&w_args));
    drop(d);
    table.push(("invocation_identity, three bindings", n));

    let (n, d) = measure(|| own.covers(own.invocation()));
    assert!(d);
    table.push(("Grant::covers, given the identity", n));

    // Building the registry is outside the claim (D12): reported, not asserted.
    let built = r.clone();
    let (n, d) = measure(|| built.with_gate(ceiling()));
    drop(d);
    println!(
        "{:<62} allocations {:>3}  bytes {:>5}  (building: not asserted)",
        "with_gate, a registry of three capabilities", n.0, n.1
    );

    let report: Vec<String> = table
        .iter()
        .map(|(what, (a, b))| format!("{what:<62} allocations {a:>3}  bytes {b:>5}"))
        .collect();
    println!("{}", report.join("\n"));
    let nonzero: Vec<&String> = table
        .iter()
        .zip(&report)
        .filter(|((_, (a, _)), _)| *a != 0)
        .map(|(_, line)| line)
        .collect();
    assert!(
        nonzero.is_empty(),
        "rows that allocate:\n{}",
        nonzero
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
}
