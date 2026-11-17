# Changelog

Every change to the crate's public API, newest first. Each entry names the Core commit the crate certifies against
(`core-pin.json`), and whether that Core version is released.

## 0.0.5-alpha.1, unchanged — 2026-11-17 — the gate at exactly the declared minimum, 1.85.0

No change to the public API, to any answer, or to what the crate certifies against (Core `a9592b7`, `0.43.0`,
released); the version stays `0.0.5-alpha.1` (D11.8: the declared minimum did not rise). DECISIONS.md D16.

**Checked:** `rust-version = "1.85"` is now checked at exactly 1.85.0, the version it admits, rather than at 1.85.1, to
which rustup resolves the name `1.85`. At rustc 1.85.0 the crate builds with every feature, without `std` and for
`thumbv7em-none-eabihf`, and passes every test, the conformance legs and the allocation table among them, in debug and
in release, hosted and without `std`. The declared minimum stays `1.85`.

**Gate:** CI's minimum job installs the floor read from `Cargo.toml` (a missing patch component read as `.0`) and fails
unless the compiler that ran its legs reports exactly that version. `run.ps1` chooses the minimum's toolchain by the
version its compiler reports, not by its name; when the floor is missing it names `1.85.0` in the remedy, and when only
another patch of 1.85 is installed it runs the legs there and says they are not the minimum.

## 0.0.5-alpha.1, unchanged — 2026-11-17 — the gate at the declared minimum Rust version

No change to the public API, to any answer, or to what the crate certifies against (Core `a9592b7`, `0.43.0`,
released); the version stays `0.0.5-alpha.1` (D11.8: the declared minimum did not rise). DECISIONS.md D15.

**Checked:** the declared minimum, `rust-version = "1.85"` (rustc 1.85.1), is now run by the gate. At it the crate
builds with every feature, without `std` and for `thumbv7em-none-eabihf`, and passes every test, the conformance legs
and the allocation table among them, in debug and in release, hosted and without `std`. CI runs a job at the minimum,
read from `Cargo.toml`, beside the job at stable. The format check and the lints run at stable only: the clippy of 1.85
raises four lints that stable's does not.

**Gate:** `run.ps1` runs the minimum's legs through rustup when that toolchain is installed and names it as missing,
with the commands that install it, when it is not (`-MinimumOnly`, `-SkipMinimum`). The allocation table now runs in
release as well as in debug, on both profiles, printing every row.

## 0.0.5-alpha.1 — 2026-11-16 — breaking

Follows Core `a9592b7ff6ab7ed7d2e9ba83bbe6b13e163ce35e`, version `0.43.0`, **released** (the commit tagged `v0.43.0`; the
pin does not move). The crate gains the query declaration of Core's Phase 46 and 398 with the query registry's grant of
Phase 651 (the reference's D261), and answers `queryGrantLaws`. DECISIONS.md D14 records the design, its alternatives
and the measured costs.

**Breaking:**

- `PolicyGate<C>` is now `PolicyGate<C, D = Capability, A = [(String, String)]>`, the gate over a declaration `D` and
  its argument list `A`; at its defaults it is the capability gate it was. `PolicyGate::new` and `ignoring_context`
  take a closure over `&D` and `&A`, so a closure whose declaration parameter is unannotated, built where nothing fixes
  its type (`let g = PolicyGate::new("p", |_: &u8, c, _| … c.id …)`, then `with_gate(g)`), no longer infers (`E0282`):
  annotate the parameter (`c: &Capability`) or build the gate where it is passed. A gate passed straight to
  `with_gate`, or built where its type is declared, is unchanged.

**Added:**

- The module `query`: `Query`, `QueryParam`, `ColumnPredicate`, `SortDirection` and `SortKey` as plain data;
  `Query::decode` (the lenient declaration reader, the filter and the order read through the seams' reader) and
  `Query::encode` (the canonical text); `Query::admission_fault_at` and `admission_fault`, the admission gate the
  reader and the registry run; `Query::validate_params`; the read's identity (`invocation_identity`,
  `write_invocation_identity`) and its grant (`Query::grant`); `QueryError`, the reference's closed union with
  `describe`; and `QueryRegistry<C = ()>` with `QueryGate<C>`, the capability registry's twin in every verb it carries
  (`register`, `try_find`, `enumerate`, `ids`, `unregister`, `with_gate`, `gates`, `resolve`, `decide_named`,
  `dispatch`, `under`).
- `Grant::covers_read`: whether a grant is for exactly a read, compared with the identity as it is written.

**Behaviour:** a read's grant is decided by the function the capability registry decides by (the gates' join, the
first gate of the most refusing rank, a `NeedsApproval` lifted only by a grant for exactly the read), so one function
decides at both registries. The read's identity is the query id, the filter's and the order's canonical text, then each
binding's name, cell tag and payload in name order, with no hash; a cell is identified by its tag, and no page token is
in it, so a grant covers every page of the read. Nothing the capability registry decides, keys or spells changed.

**Allocation:** the zero-allocation table of `0.0.3-alpha.1` holds for every row, and a read's decision, dispatch and
grant check at the query registry are zero too, for a filtered read as for one without, because the registry renders a
query's filter and order text when it registers it. Asked of a declaration alone, a filtered read's identity renders
that text. Measured in D14.8.

**Certification:** `queryGrantLaws` 19 of 19 (`invocationIdentity` 11, `grantCovers` 8), on the standard profile and
again without `std`; `capabilityLaws` 144, `capabilityDocumentLaws` 18 and `capabilityGateLaws` 41 unchanged; every
other family the crate answered before, unchanged. Core emits no vector for the declaration reader's or the admission
gate's refusals; `tests/query.rs` pins each to the reference's text at the pin. `identity` stays named and unanswered
(D14.7), as does `encodingProfile` (D8).

## 0.0.4-alpha.1 — 2026-11-16 — breaking

Follows Core `a9592b7ff6ab7ed7d2e9ba83bbe6b13e163ce35e`, version `0.43.0`, **released** (the commit tagged `v0.43.0`;
the previous pin was the `0.43.0` draft at `c7d3013`). The value spaces gain the document space of Core's Phase 644.
DECISIONS.md D13 records the design, the families the crate answers and does not at this commit, and the measured
costs.

**Breaking:**

- `ValueSpace` gains the case `Document(Option<String>)`: a well-formed JSON document, of the named type when one is
  given (the name is carried and compared, never interpreted). `ValueSpace` is not `#[non_exhaustive]`, so an
  exhaustive `match` over it no longer compiles until it handles the case.

**Added:**

- `capability::document_of`: a document argument's one spelling, its cross-host canonical text, or `None` for a
  string that is no document.
- A document space on the wire: `{"$type":"document"}` with an optional `documentType`, read and written by the space
  codec, the declaration codec, the declared result and the pipeline seam (a node may declare a document output).

**Behaviour of the new case:** a string lies in a document space when the strict reader reads it and it holds no
`null` (so no repeated member, no ill-formed string and no number a finite double cannot spell); one that does not is
`ArgOutOfSpace`, described as "a document: well-formed JSON". The capture key, the identity and a grant's coverage
write a document argument as its canonical text, so two spellings of one document are one key and one grant. In the
relation the documents are a family of their own. Nothing any other space decides, keys or spells changed.

**Allocation:** the zero-allocation table of `0.0.3-alpha.1` holds for every row. A document argument allocates to be
read, in validation and in a grant's check, in proportion to the document; measured in D13.8.

**Certification:** `capabilityDocumentLaws` 18 of 18, `capabilityLaws` 144 and `capabilityGateLaws` 41, each on the
standard profile and again without `std`; every other family the crate answered before, unchanged. `queryGrantLaws`
(the identity of a query read and its grant) is not answered: the crate holds no query declaration (D13.5).
`tests/families.rs` names every family the pinned Core emits as answered or not, with the reason.

## 0.0.3-alpha.1 — 2026-11-14 — breaking

Follows Core `c7d3013cfed62f2a03c2651098b5440b9ff8abcf`, version `0.43.0`, which is a **draft and not yet released**
(the pin did not move). The capability decision path allocates nothing once the registry is built: `decide_named` and
`dispatch`, allowed, needing approval, granted, parked or denied, held at zero allocations by `tests/capability_alloc.rs`
on the embedded profile and the hosted one. Nothing a gate decides, a grant covers or the identity holds changed.
DECISIONS.md D12 records the design and its measured costs.

**Breaking:**

- `CapabilityRegistry::decide_named` answers the deciding gate's name borrowed from the registry:
  `Result<(&str, PolicyDecision), InvokeError>` (was `(String, PolicyDecision)`).
- `PolicyDecision::Deny` carries shared text: `message: Arc<str>`, `alternatives: Arc<[Arc<str>]>` (were `String` and
  `Vec<String>`). `PolicyDecision::deny` takes `impl Into<Arc<str>>`, and `deny_with` takes that and
  `impl Into<Arc<[Arc<str>]>>` (a `Vec<Arc<str>>` or an array of them). A gate that builds its guidance once and clones
  it into each denial allocates nothing when it denies.
- `InvokeError::PolicyRefused` carries `policy: Arc<str>`, `reason: Arc<str>`, `allowed: Arc<[Arc<str>]>`, and
  `InvokeError::ApprovalRequired` carries `policy: Arc<str>`: the gate's own name and guidance, shared, not copied.
- `PolicyGate::new` and `PolicyGate::ignoring_context` take the name as `impl Into<Arc<str>>`; a `&str` or a `String`
  still converts.

**Added:**

- `Capability::write_invocation_identity`: the invocation's identity written through a `core::fmt::Write`, building
  nothing, for a host that writes it into a buffer of its own.
- `Grant::covers_invocation`: whether a grant covers an invocation, compared with the identity as it is written, which
  is what a dispatch now asks.

**Changed, with no change to any answer:** `ValueSpace::validate` and argument validation decide membership on the
value, without spelling it, and build a refusal only when they answer one; `invocation_identity` allocates exactly its
answer; `invocation_key` writes the same fields. A tree argument's membership still reads its document (D12.2).

**Certification:** unchanged counts: `capabilityLaws` 144 and `capabilityGateLaws` 41, each on the standard profile and
again without `std`, the identity and a grant's coverage now answered in the written forms and asserted equal to the
built ones.

## 0.0.2-alpha.1 — 2026-11-14 — breaking

Follows Core `c7d3013cfed62f2a03c2651098b5440b9ff8abcf`, version `0.43.0`, which is a **draft and not yet released**
(Core Phases 638, 642 and 650; the reference's D249, D253 and D255). DECISIONS.md D11 records the design.

**Breaking:**

- `Capability` gains `result: Returns` (what it answers: `Returns::Nothing` or `Returns::Value(space)`), and
  `Capability::new` takes it as its fourth argument. A declaration's wire document carries the member `result`; the
  reader refuses one without it (`MissingField` at `result`) or with an ill-formed one (`OutOfRange` at `result`).
- `admission_of` takes the declared result.
- `DeclarationFault` gains `EmptyResult` and `NonFiniteResult`; a `match` over it must cover them.
- `CapabilityRegistry` is generic over its decision context, `CapabilityRegistry<C = ()>`; `CapabilityRegistry::new()`
  is the registry at `()`, and `CapabilityRegistry::empty()` builds one for any context type. It no longer derives
  `PartialEq` structurally: two registries are equal when they hold the same capabilities and the same gates (by name
  and by identity of the decision function).

**Added:**

- The gate: `PolicyDecision`, `PolicyGate<C>` (`new`, `ignoring_context`), `CapabilityRegistry::with_gate`, `gates`,
  `decide_named`, `dispatch` and `under`.
- The grant: `Grant` (`of_invocation`, `invocation`, `covers`), `Capability::invocation_identity` (the invocation's
  injective identity, no hash taken) and `Capability::grant`.
- The pipeline type check: `CapabilityPipeline` (`decode`, `type_check`), `PipelineNode`, `ArgSource`, `PipelineError`,
  and `ValueSpace::subsumes`.
- `result_of` and `result_to_json`, the declared result's reader and writer.

**Certification:** `capabilityLaws` 144 vectors (was 72) and the new `capabilityGateLaws` 41 vectors, each on the
standard profile and again without `std`; `seamRefusals` 142 (was 138), `skeletonDag` 12 (was 11), `skeletonMerge` 13
(was 12).

## 0.0.1-alpha.1

The first certification (DECISIONS.md D1 to D10).
