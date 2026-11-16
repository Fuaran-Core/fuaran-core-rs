# Changelog

Every change to the crate's public API, newest first. Each entry names the Core commit the crate certifies against
(`core-pin.json`), and whether that Core version is released.

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
