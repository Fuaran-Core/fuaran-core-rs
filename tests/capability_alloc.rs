//! The capability decision path allocates nothing once the registry is built
//! (DECISIONS.md D12): every row of the table D11.7 measured, asserted at zero,
//! with a counting global allocator around exactly the call under test; and a
//! read's, decided by the same function at the query registry (D14).
//!
//! A global allocator is process-wide, so this leg is its own test binary and
//! counts on the measuring thread only: the harness's own threads allocate
//! while a test runs, and a process-wide count would read them. The window is
//! armed after the registry, the arguments and the grant are built, and
//! disarmed before the call's answer is dropped. The first row is a known
//! allocation, so the count is shown to count before any row is read as zero.
//!
//! `run.ps1` runs it on the hosted profile (in `cargo test --all-features`) and
//! on the embedded one (the library built without `std`), in debug and in
//! release, printing every row, and again at the minimum Rust version the
//! manifest declares (DECISIONS.md D15); it runs on the build host, because no
//! emulator is in the gate.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::fmt::{self, Write};
use std::hint::black_box;
use std::sync::Arc;

use fuaran_core::capability::{
    Capability, CapabilityRegistry, InvokeError, PolicyDecision, PolicyGate,
};
use fuaran_core::column::Cell as QCell;
use fuaran_core::query::{Query, QueryError, QueryRegistry};

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
    let message: Arc<str> = "erasing is not open to this context".into();
    let alternatives: Arc<[Arc<str>]> = [Arc::from("read")].into();
    PolicyGate::new(
        "ceiling",
        move |ctx: &u8, c: &Capability, _: &[(String, String)]| match (*ctx, c.id.as_str()) {
            (UNTRUSTED, "write") => PolicyDecision::NeedsApproval,
            (UNTRUSTED, "erase") => PolicyDecision::Deny {
                message: Arc::clone(&message),
                alternatives: Arc::clone(&alternatives),
            },
            _ => PolicyDecision::Allow,
        },
    )
}

/// A registry of the given declarations, with no gate.
fn registry_of(declarations: &[&str]) -> CapabilityRegistry<u8> {
    declarations
        .iter()
        .fold(CapabilityRegistry::empty(), |r, text| {
            r.register(Capability::decode(text).expect("a declaration"))
                .expect("admitted")
        })
}

fn registry() -> CapabilityRegistry<u8> {
    registry_of(&[WRITE, READ, ERASE]).with_gate(ceiling())
}

/// A text of at most `N` bytes on the stack: where a host with no allocation
/// to spare writes an identity.
struct StackText<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> StackText<N> {
    fn new() -> Self {
        StackText {
            bytes: [0; N],
            len: 0,
        }
    }

    fn as_bytes(&self) -> &[u8] {
        &self.bytes[..self.len]
    }
}

impl<const N: usize> Write for StackText<N> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let end = self.len + s.len();
        let room = self.bytes.get_mut(self.len..end).ok_or(fmt::Error)?;
        room.copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
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
    // The same invocation, its bindings in another order and its numbers in
    // other spellings: the grant minted for `w_args` is for it too.
    let w_respelled = bindings(&[("s", "a"), ("x", "1.50"), ("n", "07")]);
    let own = write.grant(&w_args);
    let other = write.grant(&bindings(&[("n", "8"), ("x", "1.5"), ("s", "a")]));
    let guidance: Arc<str> = "an erase is held for the audit".into();

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
    assert_eq!(d, Ok(("", PolicyDecision::Allow)));
    table.push(("decide_named, allowed, one binding", n));

    let (n, d) = measure(|| r.decide_named(&UNTRUSTED, None, "write", &w_args));
    assert_eq!(d, Ok(("ceiling", PolicyDecision::NeedsApproval)));
    table.push(("decide_named, needs approval, no grant, three bindings", n));

    let (n, d) = measure(|| r.decide_named(&UNTRUSTED, Some(&own), "write", &w_args));
    assert_eq!(d, Ok(("ceiling", PolicyDecision::Allow)));
    table.push((
        "decide_named, needs approval, its own grant, three bindings",
        n,
    ));

    let (n, d) = measure(|| r.decide_named(&UNTRUSTED, None, "erase", &e_args));
    assert!(matches!(d, Ok(("ceiling", PolicyDecision::Deny { .. }))));
    drop(d);
    table.push(("decide_named, denied, one binding", n));

    let (n, d) = measure(|| r.dispatch(&TRUSTED, None, "read", &r_args, |_| Ok(())));
    assert_eq!(d, Ok(()));
    table.push(("dispatch, allowed, one binding", n));

    let (n, d) = measure(|| r.dispatch(&UNTRUSTED, Some(&own), "write", &w_args, |_| Ok(())));
    assert_eq!(d, Ok(()));
    table.push(("dispatch, granted, three bindings", n));

    let (n, d) = measure(|| r.dispatch(&UNTRUSTED, Some(&own), "write", &w_respelled, |_| Ok(())));
    assert_eq!(d, Ok(()));
    table.push(("dispatch, granted, out of address order and respelled", n));

    let (n, d) = measure(|| r.dispatch(&UNTRUSTED, None, "write", &w_args, |_| Ok(())));
    assert!(matches!(d, Err(InvokeError::ApprovalRequired { .. })));
    drop(d);
    table.push(("dispatch, parked (approval required), three bindings", n));

    let (n, d) = measure(|| r.dispatch(&UNTRUSTED, None, "erase", &e_args, |_| Ok(())));
    assert!(matches!(d, Err(InvokeError::PolicyRefused { .. })));
    drop(d);
    table.push(("dispatch, denied, one binding", n));

    // The identity: written into a host's own buffer, which is what the embedded
    // profile offers in place of the `String` (D12), and compared with a grant
    // as it is written.
    let (n, d) = measure(|| {
        let mut buf = StackText::<64>::new();
        read.write_invocation_identity(&r_args, &mut buf)
            .map(|()| buf)
    });
    assert_eq!(d.expect("fits").as_bytes(), b"read\x01n\x013\x01");
    table.push((
        "invocation identity written to a stack buffer, one binding",
        n,
    ));

    let (n, d) = measure(|| {
        let mut buf = StackText::<64>::new();
        write
            .write_invocation_identity(&w_args, &mut buf)
            .map(|()| buf)
    });
    assert_eq!(d.expect("fits").as_bytes(), own.invocation().as_bytes());
    table.push((
        "invocation identity written to a stack buffer, three bindings",
        n,
    ));

    let (n, d) = measure(|| own.covers_invocation(&write, &w_respelled));
    assert!(d);
    table.push(("Grant::covers_invocation, its own grant, three bindings", n));

    let (n, d) = measure(|| other.covers_invocation(&write, &w_args));
    assert!(!d);
    table.push(("Grant::covers_invocation, another grant, three bindings", n));

    let (n, d) = measure(|| own.covers(own.invocation()));
    assert!(d);
    table.push(("Grant::covers, given the identity", n));

    let (n, d) = measure(|| PolicyDecision::deny(Arc::clone(&guidance)));
    assert!(matches!(d, PolicyDecision::Deny { ref alternatives, .. } if alternatives.is_empty()));
    drop(d);
    table.push(("a denial of shared guidance, no alternatives", n));

    // The identity as a `String` allocates its answer and nothing else, sized
    // exactly: one allocation of the identity's length.
    let (n, d) = measure(|| write.invocation_identity(&w_args));
    assert_eq!(d, own.invocation());
    assert_eq!(
        n,
        (1, d.len()),
        "invocation_identity: its answer and nothing else"
    );
    drop(d);

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

// ─── A document argument (DECISIONS.md D13) ──────────────────────────────────

/// The document family's capability: one required hole `d0` in the document
/// space of type `Order`, answering a `Receipt`.
const DOCUMENT: &str = r#"{"$type":"capability","determinism":"clock","id":"cap-document","placement":{"$type":"server"},"result":{"$type":"document","documentType":"Receipt"},"signature":{"effect":{"determinism":"clock","host":"readsHost"},"holes":[{"addr":"d0","kind":"value","name":"order","required":true,"space":{"$type":"document","documentType":"Order"}}],"name":"capDocument"}}"#;

/// A registry holding the document capability beside `read`, under a gate that
/// parks the document for approval in the untrusted context.
fn document_registry() -> CapabilityRegistry<u8> {
    registry_of(&[DOCUMENT, READ]).with_gate(PolicyGate::new(
        "review",
        |ctx: &u8, c: &Capability, _: &[(String, String)]| match (*ctx, c.id.as_str()) {
            (UNTRUSTED, "cap-document") => PolicyDecision::NeedsApproval,
            _ => PolicyDecision::Allow,
        },
    ))
}

/// A document argument is read to decide its membership, and read and written
/// to spell it in the identity: what forces those allocations is the strict
/// reader (the value it builds, its nesting stack and the member names it
/// remembers to refuse a repeated one), the `null` walk's stack, and the
/// canonical text. So the rows are REPORTED, each shown to allocate, and a
/// decision on a registry that holds a document capability still allocates
/// nothing for a capability with no document hole.
#[test]
fn a_document_argument_allocates_to_read_its_document() {
    let r = document_registry();
    let doc = r.try_find("cap-document").expect("cap-document").clone();
    let canonical = bindings(&[("d0", r#"{"a":1,"b":[true,"x\u000ay",2.5]}"#)]);
    let respelled = bindings(&[(
        "d0",
        r#"{ "b" : [ true, "x\ny", 2.50 ],
  "a" : 1 }"#,
    )]);
    let not_a_document = bindings(&[("d0", "order")]);
    let r_args = bindings(&[("n", "3")]);
    let own = doc.grant(&canonical);

    // The calibration row, as in the table above.
    let (known, v) = measure(|| Vec::<u8>::with_capacity(16));
    drop(v);
    assert_eq!(known, (1, 16), "the counting allocator counts");

    // No document hole: nothing, though the registry holds a document capability.
    let (n, d) = measure(|| r.decide_named(&TRUSTED, None, "read", &r_args));
    assert_eq!(d, Ok(("", PolicyDecision::Allow)));
    assert_eq!(n, (0, 0), "a decision with no document argument");

    let mut rows: Vec<(&str, (usize, usize))> = Vec::new();

    let (n, d) = measure(|| doc.validate_args(&canonical));
    assert_eq!(d, Ok(()));
    rows.push(("validate_args, one document (canonical spelling)", n));

    let (n, d) = measure(|| doc.validate_args(&respelled));
    assert_eq!(d, Ok(()));
    rows.push(("validate_args, one document (another spelling)", n));

    let (n, d) = measure(|| r.decide_named(&TRUSTED, None, "cap-document", &canonical));
    assert_eq!(d, Ok(("", PolicyDecision::Allow)));
    rows.push(("decide_named, allowed, one document", n));

    let (n, d) = measure(|| r.decide_named(&UNTRUSTED, Some(&own), "cap-document", &respelled));
    assert_eq!(d, Ok(("review", PolicyDecision::Allow)));
    rows.push(("decide_named, its own grant, the document respelled", n));

    let (n, d) = measure(|| r.dispatch(&TRUSTED, None, "cap-document", &canonical, |_| Ok(())));
    assert_eq!(d, Ok(()));
    rows.push(("dispatch, allowed, one document", n));

    let (n, d) = measure(|| own.covers_invocation(&doc, &respelled));
    assert!(d);
    rows.push(("Grant::covers_invocation, the document respelled", n));

    let (n, d) = measure(|| {
        let mut buf = StackText::<96>::new();
        doc.write_invocation_identity(&respelled, &mut buf)
            .map(|()| buf)
    });
    assert_eq!(d.expect("fits").as_bytes(), own.invocation().as_bytes());
    rows.push((
        "invocation identity written to a stack buffer, one document",
        n,
    ));

    let (n, d) = measure(|| doc.validate_args(&not_a_document));
    assert!(matches!(d, Err(InvokeError::ArgOutOfSpace { .. })));
    drop(d);
    rows.push(("validate_args, refused: no document", n));

    let report: Vec<String> = rows
        .iter()
        .map(|(what, (a, b))| format!("{what:<62} allocations {a:>3}  bytes {b:>5}"))
        .collect();
    println!("{}", report.join("\n"));
    for ((what, (a, _)), line) in rows.iter().zip(&report) {
        assert!(*a > 0, "{what} reads its document, so it allocates: {line}");
    }
}

// ─── A read (DECISIONS.md D14) ───────────────────────────────────────────────

/// Two of `queryGrantLaws`' queries: `ledger`, which declares no filter and no
/// order, and `shaped`, which declares both.
const LEDGER: &str = r#"{"$type":"query","effect":{"determinism":"network","host":"readsHost"},"id":"ledger","pageSize":50,"params":[{"name":"n","required":true,"type":"int"}],"resultSchema":[{"name":"n","type":"int"},{"name":"region","type":"string"}],"source":{"ref":"ledger","schema":[]}}"#;
const SHAPED: &str = r#"{"$type":"query","effect":{"determinism":"network","host":"readsHost"},"id":"shaped","orderBy":[{"column":"n","direction":"descending"}],"pageSize":50,"params":[{"name":"n","required":true,"type":"int"}],"resultSchema":[{"name":"n","type":"int"},{"name":"region","type":"string"}],"source":{"ref":"shaped","schema":[]},"where":[{"$type":"contains","column":"region","text":"eu"},{"$type":"atLeast","column":"n","type":"int","value":2}]}"#;

/// A query registry over a `u8` context: every read is open to the trusted
/// context; in the untrusted one `ledger` and `shaped` need an approval.
fn query_registry() -> QueryRegistry<u8> {
    [LEDGER, SHAPED]
        .iter()
        .fold(QueryRegistry::empty(), |r, text| {
            r.register(Query::decode(text).expect("a declaration"))
                .expect("admitted")
        })
        .with_gate(PolicyGate::new(
            "reads",
            |ctx: &u8, _: &Query, _: &[(String, QCell)]| match *ctx {
                TRUSTED => PolicyDecision::Allow,
                _ => PolicyDecision::NeedsApproval,
            },
        ))
}

/// A read is decided through the capability registry's own decision (D14.6),
/// so the claim D12 makes for a capability holds for a read: once the registry
/// is built, deciding and dispatching allocate nothing, for a read whose
/// declaration filters and orders as for one that does not, because the
/// registry rendered that text when it registered the query. Asserted at zero.
/// Two rows outside the registry are REPORTED: the identity and a grant's
/// coverage of a filtered read asked of the declaration alone render its filter
/// and order text, which is what allocates there.
#[test]
fn a_reads_decision_allocates_nothing_once_the_registry_is_built() {
    let r = query_registry();
    let ledger = r.try_find("ledger").expect("ledger").clone();
    let shaped = r.try_find("shaped").expect("shaped").clone();
    let args = vec![("n".to_string(), QCell::Int(5))];
    let own_ledger = ledger.grant(&args);
    let own_shaped = shaped.grant(&args);
    let other = ledger.grant(&[("n".to_string(), QCell::Int(6))]);

    let (known, v) = measure(|| Vec::<u8>::with_capacity(16));
    drop(v);
    assert_eq!(known, (1, 16), "the counting allocator counts");

    let mut table: Vec<(&str, (usize, usize))> = Vec::new();

    let (n, d) = measure(|| ledger.validate_params(&args));
    assert_eq!(d, Ok(()));
    table.push(("read: validate_params, one binding", n));

    let (n, d) = measure(|| r.decide_named(&TRUSTED, None, "ledger", &args));
    assert_eq!(d, Ok(("", PolicyDecision::Allow)));
    table.push(("read: decide_named, allowed", n));

    let (n, d) = measure(|| r.decide_named(&UNTRUSTED, None, "ledger", &args));
    assert_eq!(d, Ok(("reads", PolicyDecision::NeedsApproval)));
    table.push(("read: decide_named, needs approval, no grant", n));

    let (n, d) = measure(|| r.decide_named(&UNTRUSTED, Some(&own_ledger), "ledger", &args));
    assert_eq!(d, Ok(("reads", PolicyDecision::Allow)));
    table.push(("read: decide_named, its own grant", n));

    let (n, d) = measure(|| r.decide_named(&UNTRUSTED, Some(&own_shaped), "shaped", &args));
    assert_eq!(d, Ok(("reads", PolicyDecision::Allow)));
    table.push(("read: decide_named, its own grant, filtered and ordered", n));

    let (n, d) = measure(|| r.decide_named(&UNTRUSTED, Some(&other), "ledger", &args));
    assert_eq!(d, Ok(("reads", PolicyDecision::NeedsApproval)));
    table.push(("read: decide_named, another read's grant", n));

    let (n, d) = measure(|| r.dispatch(&UNTRUSTED, Some(&own_shaped), "shaped", &args, |_| Ok(())));
    assert_eq!(d, Ok(()));
    table.push(("read: dispatch, granted, filtered and ordered", n));

    let (n, d) = measure(|| r.dispatch(&UNTRUSTED, None, "ledger", &args, |_| Ok(())));
    assert!(matches!(d, Err(QueryError::QueryApprovalRequired { .. })));
    drop(d);
    table.push(("read: dispatch, parked (approval required)", n));

    let (n, d) = measure(|| {
        let mut buf = StackText::<64>::new();
        ledger
            .write_invocation_identity(&args, &mut buf)
            .map(|()| buf)
    });
    assert_eq!(
        d.expect("fits").as_bytes(),
        own_ledger.invocation().as_bytes()
    );
    table.push(("read: identity written to a stack buffer, no filter", n));

    let (n, d) = measure(|| own_ledger.covers_read(&ledger, &args));
    assert!(d);
    table.push(("read: Grant::covers_read, no filter", n));

    let report: Vec<String> = table
        .iter()
        .map(|(what, (a, b))| format!("{what:<62} allocations {a:>3}  bytes {b:>5}"))
        .collect();
    println!("{}", report.join("\n"));

    // Outside the registry, a filtered read's text is rendered where it is asked.
    let (n, d) = measure(|| own_shaped.covers_read(&shaped, &args));
    assert!(d);
    println!(
        "{:<62} allocations {:>3}  bytes {:>5}  (reported: renders the filter and order)",
        "read: Grant::covers_read, filtered and ordered", n.0, n.1
    );
    assert!(n.0 > 0, "the filter and order text is rendered");
    let (n, d) = measure(|| {
        let mut buf = StackText::<256>::new();
        shaped
            .write_invocation_identity(&args, &mut buf)
            .map(|()| buf)
    });
    assert_eq!(
        d.expect("fits").as_bytes(),
        own_shaped.invocation().as_bytes()
    );
    println!(
        "{:<62} allocations {:>3}  bytes {:>5}  (reported: renders the filter and order)",
        "read: identity to a stack buffer, filtered and ordered", n.0, n.1
    );

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
