# Changelog

Every change to the crate's public API, newest first. Each entry names the Core commit the crate certifies against
(`core-pin.json`), and whether that Core version is released.

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
