# Decisions

Each entry records a decision about this crate, why it was taken, and what it rules out.

## D1 — 2026-10-23 — the repository, its organisation and its licence

`fuaran-core-rs` lives at `Fuaran-Core/fuaran-core-rs`, beside the reference implementation it twins
(`Fuaran-Core/fuaran-core`), and is Apache-2.0 and public from its first commit, for the reason the
reference is: a spine every host builds on is only useful if every host can hold it. (Operator ruling,
recorded by Phase 463.)

## D2 — 2026-10-23 — the crate's identity, and how a host depends on it

- The package is `fuaran-core` and its library `fuaran_core`. `fuaran-core` is the intended crates.io
  name (unclaimed on 2026-10-23); `publish = false` holds until a release is a deliberate act, so
  nothing is published by this decision.
- A host depends on it as `fuaran-core = { version = "<crate version>", path = "../fuaran-core-rs",
  default-features = false }`: the path is the working layout (the two repositories checked out side by
  side, in CI as locally), and the version requirement both keeps a later crates.io publish open and
  makes a version move here fail a consumer's build loudly instead of silently changing what it links.
- `no_std` + `alloc` is the default build; `std` is a feature that adds nothing today and exists so a
  hosted consumer can ask for it without a breaking change later. No dependency is taken.

## D3 — 2026-10-23 — what the spine holds, and what it does not (a premise checked)

Phase 463 was written as a MOVE of the spine out of the Rust UI host. Checked against that host, the
premise holds for part of it and not for the rest:

- **Moved, verbatim in behaviour:** canonical JSON (the value model, the reader, the canonical
  writer), the canonical number form, the exact decimal (with the rounding modes it computes under),
  and SHA-256. The UI host re-exports each under the paths it already published.
- **Generalised while moving:** the reader takes a limits record (`JsonLimits`) instead of one wire's
  constants, classifies malformed text by the reference's `JsonErrorKind`, and reports a breach at
  its path; the chain is the pre-image and link over OPAQUE op text, and the actor with its one
  canonical spelling, which the UI host's typed op-record chain now computes its hashes through.
- **Not moved, because there was nothing generic to move:** the UI host's apply engine works over its
  own typed node and op vocabulary, its DAG module is the codec of a UI record (it mints and verifies
  no content id), and its three-way merge is over UI trees. They stay in that host. What the spine
  holds instead is Core's own contract, written for this crate: the skeleton tree and its six ops
  (`tree`, `ops`), and the content-addressed DAG with its orders, ranks, replays and integrity walk
  (`dag`), generic over the op through `DagOp`.

## D4 — 2026-10-23 — where the vectors come from

The conformance legs read the reference's `conformance/` directory from a Core checkout at the commit
`core-pin.json` names (found at `FUARAN_CORE_DIR`, or as `fuaran-core/` beside this repository or an
ancestor of it). A missing checkout FAILS the legs: a suite that skipped its vectors would read green
having certified nothing. The pin moves only as a deliberate act, in the change that needs it, with the
legs re-run against the new commit.

## D5 — 2026-10-23 — two value models over one reader

`json::JVal` is the model a wire host reads documents into: it carries `null` (a wire answers a null by
position) and every number as a double (exact to 2^53, past which only a canonical float layout is
admitted). The decode error contract is the reference's, whose model has no null and distinguishes an
integer token within `i32` from every other number — `2.0` is not an `int` there. Both are built by ONE
reader, generic over what it builds, so the grammar, the rows and the limits are stated once; the decode
layer (`decode::Value`) refuses a null as `NullNotRepresentable`, as the reference's strict reading does.

## D6 — 2026-10-23 — what the first certification covers

*Superseded in part by D8 (2026-10-24):* the table below is Phase 463's first certification as it stood. What the
spine runs now, and every count, is D8 and the spine's adoption record in each of the reference's family manifests
(`fuaran-core-rs`); this entry holds no current count.

Certified against the reference's vectors (`tests/conformance.rs`), every vector of each set named:

| Family | Run | Not run, and why |
|---|---|---|
| `refusals/codec-refusals.json` | the 56 `json` vectors | the `column` and `profile` vectors: the columnar codec and the wire-profile reader are not the spine's |
| `refusals/seam-refusals.json` | — | the capability and query seams are not the spine's |
| `decode/decode-rejects.json` | all 47 | — |
| `laws/decimal-laws.json` | the 53 `canonical`, `compare`, `add`, `toFloat` vectors | `codecDecode`, `codecEncode`, `aggregate`: the column codec and the aggregates |
| `escape/string-escape.json` | all 38 lines | — |
| `apply/skeleton-apply.json` | all 25 | — |
| `chain/skeleton-chain.json` | all 16 | — |
| `dag/skeleton-dag.json` | all 11 | — |
| `merge/`, `diff/` | — | the reconcile and the diff are not in the spine yet |

A rejection is answered in the reference's own vocabulary (its canonical rejection bytes), not mapped to
another host's codes, because this crate IS the reference's twin.

## D7 — 2026-10-24 — what is proved directly, and what is not (Phase 486's verdict)

The question was whether the spine's parser and its limits can be proved directly — a model
checker over the Rust, rather than conformance against the reference's vectors — and at what cost.
The verdict, with the measurements behind it in [verification/README.md](verification/README.md):

- **The writer's pure functions: proved, cheaply, under Creusot — adopted.** The canonical escape
  is proved, byte for byte, to return exactly the reference proof's `quoted` of its input, against
  Pearlite twins of `WireCanon.fst`'s `esc_ch` / `escape` / `quoted` written clause for clause.
  About an hour of proof engineering, a two-second replay as the gate, the toolchain pinned in a
  Dockerfile. The proved function is a twin of `escape_string` over bytes, not the crate's function,
  and the agreement with the F\* model is by transliteration; both gaps are stated where the claim is.
- **The reader: not provable with Kani as it stands — measured, not adopted.** Every rung over a
  symbolic input, down to ONE byte, exhausts 15 GB; a concrete `[]` does not finish in seven minutes
  where a concrete `7` takes six seconds. The cost is the reader's `Vec`/`String` growth path under
  symbolic positions, not the UTF-8 validation, the symbolic bounds or the harness's counting
  allocator — each was removed in turn and the rung failed the same way. The harnesses stay in the
  tree behind a feature with their measurements, so the next attempt starts from the numbers.
- **Kani is kept for what closes**: `escape_string` over the empty and every one-byte ASCII
  string — two symbolic bytes already exhaust the memory, because anything that grows a `String`
  or a `Vec` under symbolic content does — as an opt-in `run.ps1 -Verify` leg beside the Creusot
  replay. Both legs run natively or in Docker,
  skip with a named reason when neither is available, and fail when a proof does not check.

*Amended by D10 (2026-10-24): the restructured reader was measured, and it does not close a rung.* What would
change the reader's verdict is a reader restructured for checking (one arena, no
reallocation on the parse path) or a proof-carrying extraction from the reference's F\*; neither is
this phase's, and both are named under the write-up's `SUCCESSOR`.

## D8 — 2026-10-24 — the spine answers every family the reference emits, and where its counts live

*Amended by D13 (2026-11-16): the spine does not answer every family the reference emits; `tests/families.rs` names
each one it does not, with the reason (`encodingProfile`, `identity`, `queryGrantLaws` at Core `0.43.0`).*
*Amended by D14 (2026-11-16): `queryGrantLaws` is answered (`tests/query_grant.rs`), and the rule that a typed seam
value waits for a consumer no longer holds for the query declaration (D14.1).*

Phase 564 extends the certification from five families to every family the reference emits under
`conformance/`, at the Core commit `core-pin.json` names. The counts are the reference's to record: each family's
manifest carries the spine's adoption record (`fuaran-core-rs`), the law families' in their own index
(`conformance/laws/manifest.json`, the reference's D231), so neither this entry nor D6 holds a count a manifest
does not. Each leg asserts the count its manifest declares and fails on a vector it did not run.

| Leg (`tests/`) | Families | What answers them |
|---|---|---|
| `conformance.rs` | `decodeRejects`, `skeletonApply`, `skeletonChain`, `skeletonDag`; the `json` vectors of `codecRefusals`; the `canonical`, `compare`, `add` and `toFloat` vectors of `decimal`; the `escape/` lines | `json`, `decode`, `decimal`, `tree`, `ops`, `chain`, `dag` |
| `structural.rs` | `skeletonMerge`, `skeletonDiff` | `merge` (the common base and `reconcile_many` under `ops::footprint`), `diff` |
| `columnar.rs` | the `column` vectors of `codecRefusals`; the `codecDecode`, `codecEncode` and `aggregate` vectors of `decimal`; `fieldLaws` | `column`, `field`, `unit` |
| `propagation_forms.rs` | `propagationLaws`, `canonicalFormLaws`; the `profile` vectors of `codecRefusals` | `propagation`, `canonical_form`, `profile` |
| `capability.rs` | `capabilityLaws`; `capabilityDocumentLaws` (since D13) | `capability`, `hash` |
| `capability_gate.rs` | `capabilityGateLaws` (since D11) | `capability` |
| `seams.rs` | `seamRefusals` | `seam` |
| `premises.rs` | `premiseStringModel`, `premiseFloatDigits`, `premiseInt53`, `premiseSha256`, `premiseRecursionDepth` | `json`, `decode`, `number`, `sha256` |
| `families.rs` (since D13) | none: it names every family a manifest declares as answered by a leg or not answered, with the reason | the manifests |

Three rulings sit inside it.

- **Every reader is `no_std`.** Nothing new is behind `std`: where `core` lacks a float function the vectors
  need bit-exact (`sqrt` for the aggregates' standard deviation, the whole-number test), the crate carries a
  correctly rounded private one. The build for `thumbv7em-none-eabihf` is part of `run.ps1`.
- **The seams are read by one declaration-driven codec**, the twin of how the reference derives a codec per seam
  union from its IDL declaration: the unions, records and enums are tables, and one interpreter reads, refuses and
  re-encodes them, strict or lenient. Thirteen hand-written readers would restate the tables. A typed Rust value
  per seam union is not offered until a consumer needs one.
- *Amended by D10 (2026-10-24): the reader no longer recurses, and the probe runs on a 64 KiB thread.*
  **The recursion premise is answered on a stated stack, and that stack is large.** The reader recurses once per
  nesting level and refuses past 512 before it descends, so it never overflows a stack that holds 512 levels; the
  probe leg runs on a 2 MiB thread. Measured on x86_64: a debug build needs more than 1 MiB for the 512-deep
  documents and a release build more than 256 KiB (384 KiB holds it). That is above a microcontroller's stack,
  so on such a target the premise holds only under a lower depth bound (`JsonLimits::depth_only`), which the
  reference's 512 does not admit. Closing it is a reader that keeps its own stack on the heap, the restructuring
  D7 names for the model checker as well; it is not this entry's.

*Declined:* the `encoding/` table. It has no manifest, so no adoption record could hold a count, and what it
pins beyond the canonical escapes the `escape/` lines already pin is the v1 profile (the rendering of stores
minted by the reference's `0.30.0` to `0.32.0`) and a writer that keeps authored member order. The spine holds
neither, because it has never read or minted such a store. A host that must open one needs both, with the table
as its certification.

## D9 — 2026-10-24 — the proved escape is the crate's own function, and the reference records the proof

Phase 566 closes the first gap D7 states and makes the proof something the reference can count:

- **The proof checks the crate's function.** `escape_bytes` lives in the crate, in `src/json/escape.rs`, and
  `json::escape_string` is that function over its argument's UTF-8 bytes. The Creusot crate compiles that file as
  one of its own modules, with the specification attached under `cfg(creusot)`, so no build but Creusot's sees it
  and the crate still takes no dependency (D2). The file is separate because the proof crate must compile all of it
  and Creusot cannot compile the reader. The transliteration gap (D7) stands, and is stated on the reference's row.
- **No `unsafe` to save a pass.** The escaped bytes become a `String` through the checked `String::from_utf8`. They
  are well-formed by construction, and an `unsafe` conversion would make the crate's memory safety rest on an
  argument no tool checks; the crate holds no `unsafe` today. The cost is one validation pass per escaped string,
  and one stub: the two closing Kani rungs replace that conversion with the unchecked one, because its loop over
  one symbolic byte exhausts the memory on its own (measured, `verification/README.md`).
- **The reference records the proof; this repository computes its identity.** The reference's claims ladder holds
  a row for this proof (`spine-escape-quoted`), naming the toolchain identity and the replay identity — the SHA-256
  of the record's manifest, which `verification/creusot/replay-identity.ps1` defines and `run.ps1 -Verify` prints —
  and a host-compliance declaration cites the row as `proof-replayed` evidence (the reference's D233). The
  reference cannot run the prover, so what it validates is that the row is this host's proof and that the
  declaration names the toolchain and the identity the row records. A change to the proof's files moves the
  identity, and the row is re-recorded in the reference in the change that lands the new proof.
- **The proofs run in CI on demand only.** `.github/workflows/verify.yml` runs `run.ps1 -Verify` on
  `workflow_dispatch`, never on a push: building the two images costs the better part of an hour for a replay that
  takes a second.

## D10 — 2026-10-24 — the reader and the writer keep their nesting on the heap

Phase 567 closes the stack question D8 left open: the depth premise now holds on a microcontroller's stack at the
reference's 512, not only on a desktop thread.

- **Neither the reader nor the canonical writer recurses.** The reader's nesting state is an explicit stack of
  frames, one per open container (its items or members so far, the names seen, the member being read). The stack is
  ONE allocation, made when the first container opens and sized to every container the read can have open at once:
  the depth bound, or the opening brackets left in the document when there are fewer, since each open container
  consumed one. Nothing on the parse path grows it; the push asserts as much in a debug build. A breach's path is
  read off the stack, root first, where it was assembled on the way back out of the recursion. `render_canonical`
  writes into one buffer with its own explicit stack. The depth bound is still checked before a container opens,
  and still always bounded, because it now sizes that allocation.
- **Nothing a caller sees moved.** Every family stays green, and a differential run of the new reader against the
  recursive one (Phase 564's, `ad4910f`) over 1,000,000 generated documents, malformed and well-formed, under random
  limits records and through both value models, found every read, refusal (kind, offset, message, bound, path) and
  rendering identical. The same run goes red when a frame's path step is planted one off.
- **Measured stack**, x86_64, the smallest thread on which 512-deep arrays and objects read (and write back):

  | Build | Recursive reader (D8) | This reader and writer |
  |---|---|---|
  | Linux, debug | 644 KiB (arrays), 1,020 KiB (objects) | 16 KiB, the smallest thread Linux starts |
  | Linux, release | 280 KiB | 16 KiB |
  | Windows, debug | 768 KiB, 1,280 KiB | 64 KiB, the smallest thread Windows starts |
  | Windows, release | 384 KiB | 64 KiB |

  100,000 levels are refused on the same threads. The probe leg (`tests/premises.rs`) runs on 64 KiB, in debug under
  `cargo test` and in release under `run.ps1`'s release leg, and the recursive reader overflows it.
- **What still recurses is a value's drop.** `JVal` and `decode::Value` are owned trees, and the compiler's drop glue
  for them recurses once per level: dropping a 512-deep value needs 104–128 KiB in a debug build and 40 KiB in a
  release build on Linux (256 and 128 KiB on Windows). The probe hands the values it accepted back to the calling
  thread to drop, and says why. Making the teardown constant-stack means a `Drop` implementation on the public value
  types, which forbids moving a field out of a value by pattern, a breaking change to every consumer that
  destructures a value it owns. That is not this additive phase's to take, so it is recorded open for a successor
  phase to rule on.
- **Parse throughput** (release, x86_64, the median of seven alternating runs against the recursive reader):

  | Input | Recursive | Heap stack | Change |
  |---|---|---|---|
  | Core's 30 conformance documents (428 KB) | 163 MB/s | 153 MB/s | −6% |
  | one array of 100,000 numbers | 225 MB/s | 211 MB/s | −6% |
  | one array of 20,000 small objects | 77 MB/s | 82 MB/s | +6% |
  | 2,000 nine-byte objects, read one at a time | 68 MB/s | 56 MB/s | −17% |
  | 2,000 scalar documents | 217 MB/s | 193 MB/s | −11% |
  | 512-deep arrays / objects | 46 / 44 MB/s | 52 / 44 MB/s | +14% / 0% |

  The tiny documents pay the stack's one allocation, which a document that opens a container now makes. The writer,
  which no longer builds a string per sub-value, is faster on every input measured: 2.2 times on the corpus, 5.5 times on the
  512-deep objects. Taken as the price of the premise.
- **Kani: no reader rung closes, and the restructuring was not what they lacked** (D7's premise, refuted at these
  rungs). Phase 486's rungs were re-run over both readers in the same pinned image, alone, under a 14 GiB cap
  (measurements in [verification/README.md](verification/README.md)): `limits_n1_d0`, `limits_n2_d0` and
  `read_n1_d0` run out of memory over both, the heap-stack reader's model 5–26% larger in CBMC's program steps, and
  `limits_n2_d1` is unfinished at nine minutes. The depth-0 rungs never open a container, so their cost was never the
  nesting stack: it is the strings and items every read still grows under symbolic content, which is D7's own
  finding. D7's verdict for the reader stands, and what would change it is no longer "one arena" but a reader whose
  strings and items do not grow under the checker (a counted pre-pass, or a proof-carrying extraction).

## D11 — 2026-11-14 — the gate decides under a context the host chooses the type of, a grant is bound to the invocation's identity as bytes, and a capability declares its result

Phase 645 follows the reference's registry change: a gate decides under a decision context and a dispatch may present a
grant (the reference's D249, Phase 638), a capability declares what it answers (D253, Phase 642), and the gate, the
context and the grant are emitted as a law family of their own (D255, Phase 650). `core-pin.json` moves to Core
`c7d3013`, whose `0.43.0` is a draft and not released.

**D11.1 — went red first.** At the new pin, before the crate changed: every one of the 144 capability law vectors
failed (each declaration now carries `result`, which the reader read past and the writer never wrote; the
`declarationRefused` and `pipelineTypeCheck` cases had no reader), the 41 vectors of `capabilityGateLaws` had no leg
and no API to run on, and 4 seam-refusal vectors failed on the new `DeclarationFault` tags. Two vectors outside the
capability moved with the pin and failed too (D11.9).

**D11.2 — the context is a type parameter, as in the reference.** `CapabilityRegistry<C = ()>` and `PolicyGate<C>`
carry it; `decide_named` and `dispatch` take `&C` after the registry. `CapabilityRegistry::new()` is the registry that
needs no context (`C = ()`), `empty()` builds one for any `C`, and `under(ctx)` fixes a context, answering a registry
at `()` that decides exactly as this one does under `ctx`. The crate names nothing a context holds. Two alternatives
lost. A context erased to `&dyn Any` is a downcast at every gate, the shape the reference rejected as a cast at every
gate (its D249.8). A context trait the crate defines (an actor, a role) would name what a context holds, which the
reference deliberately does not. Embedded cost: none in the type; a host instantiates `C` at an integer.

**D11.3 — a gate is a name and a shared decision function.** `PolicyGate<C>` holds its name and an
`Arc<dyn Fn(&C, &Capability, &[(String, String)]) -> PolicyDecision + Send + Sync>`. A registry is persistent
(`register` and `with_gate` answer a new registry), so its gates are shared between versions, and a gate built from
data at run time (the conformance leg builds every gate from the file's table) must capture. Two gates are equal when
they have the same name and the same function (`Arc::ptr_eq`), the reference's identity of functions. Alternatives: a
`fn` pointer, which cannot capture a table; `Rc`, which makes every registry `!Send` for a threaded host; a trait of the
crate's, the same cost with more ceremony. `Send + Sync` costs a gate nothing it should hold: a decision is total and
free of side effects, and a fact that changes is the host's to put in the context. Embedded cost: one allocation per
gate when it is added, a virtual call per gate per decision, and pointer-width atomics, which `thumbv7em-none-eabihf`
has; a target without them (`thumbv6m-none-eabi`) has no `alloc::sync::Arc`, so the crate no longer builds there (stated from the
`alloc` documentation, not built here: that target is not installed); an `Rc` selected by `cfg(target_has_atomic)` is the
remedy if such a host appears. The reference's denial observers are not carried: a
refusal is the dispatch's `Err`, where a host that audits reads it; no vector or consumer needs one, and one is
additive later.

**D11.4 — a grant is the invocation's identity, compared as bytes.** `Capability::invocation_identity(args)` is the
reference's `invocationPreimage`: the capability id, then each binding's address and its value's canonical spelling in
its hole's space, sorted by address (ordinal), through `hash::canonical_fields` (every U+0010 and U+0001 a field holds
preceded by U+0010, each field terminated by U+0001), with no hash. `Grant` holds that string,
`Capability::grant(args)` mints the grant for exactly this invocation, and `Grant::covers` compares the two
identities' bytes, with no normalisation. The capture key and the identity share one helper (`spelled_bindings`), so
the canonical spelling cannot drift between them, and the key keeps its bytes. A dispatch presents `Option<&Grant>`.
Three alternatives lost. The capture key (`invocation_key`), which ends in a 32-bit hash whose second pre-image is
cheap: an approval bound to it is forgeable (the reference's D249.3). A digest of the identity, which a host can take
itself (D249.8). A list of grants, which the reference takes only for a pipeline run, not ported here (D11.6). Embedded
cost: the identity is a heap string of the invocation's size (20 bytes for the family's three-binding call), rendered
only when a gate asks for an approval.

**D11.5 — one judgement under the named read and the dispatch.** `decide_named` and `dispatch` run the same private
`judge`: resolve the id (`NoSuchCapability` naming the held ids), validate the arguments, then the join over the gates
in order, the deciding gate the FIRST of the greatest rank and `""` when every gate allows, with a `NeedsApproval`
lifted to `Allow` when the grant covers this invocation and its name kept (the reference's D255.5, D255.6). So a
dispatch refuses exactly as the read decides: `PolicyRefused` with the gate, the message and the alternatives for a
`Deny`, which no grant lifts, and `ApprovalRequired` naming the gate for a needs-approval no grant lifts. The
performer is `FnOnce(&Capability) -> Result<T, InvokeError>`, so it runs at most once by type and exactly once on
`Allow`, and never on a refusal. The reference's `Deferred` envelope is not ported: the crate holds no such type, and a
host's `T` can be its own. The reference's `decide` (a refusal folded into a `Deny` with its sentence) is not ported
either: `decide_named` answers the same decision with the typed refusal.

**D11.6 — the declared result, its wire member and its refusals.** `Returns` is `Nothing` or `Value(ValueSpace)`, and
`Capability` gains `result` (`Capability::new(id, signature, placement, result)`; the record breaks every full
literal). On the wire it is the member `result`, a value space's own document or `{"$type":"nothing"}`. The reader
requires it (`MissingField` at `result`) and holds it to the admission gate after the signature, so a signature fault
is answered first (`OutOfRange` at `result`); the registry refuses it at `register`; the guarded encode refuses a
non-finite bound in it. `DeclarationFault` gains `EmptyResult` and `NonFiniteResult`, and the seam table gains their
tags after the hole faults. The pipeline type check is ported because the capability family's `pipelineTypeCheck`
vectors test the result's one checked use, a call's declared output: `CapabilityPipeline` (read through the pipeline
seam's own declaration, then typed), `PipelineError` and `type_check`, the whole of the reference's check, with
`ValueSpace::subsumes`, the relation it asks. Two alternatives lost: leaving the 24 vectors unrun, which would adopt the
family at 120 of 144 and leave the result unchecked anywhere; and checking the declared output alone, a partial check
answering under the reference's name for pipelines the reference refuses. Not ported, with no vector or consumer:
evaluation, a node's capture key, the pipeline's writer, and `PipelineError`'s sentences. Embedded cost: none beyond the
declaration's own heap.

**D11.7 — the embedded profile, measured: the context can be an integer, and the decision path allocates.** The crate
is `no_std` + `alloc` on every profile and the `std` feature adds nothing, so the embedded profile is the library built
without `std`, for `thumbv7em-none-eabihf`. `run.ps1` now runs the capability law and gate legs a second time with
`--no-default-features`; both are green. They run on the build host: the target build only builds, because no emulator
is in the gate. Every gate vector is decided twice, under the file's string labels and under a `u8` context (each
label's index), so "a gate decides on a context of integers only" holds. "And no allocation" does not hold for the crate
as it stands, measured with a counting global allocator around one call (x86_64; release and debug equal):

| Call | Allocations | Bytes |
|---|---|---|
| a gate's own decision over a `u8` context, `Allow` or `NeedsApproval` | 0 | 0 |
| `Grant::covers`, given the identity | 0 | 0 |
| `decide_named`, allowed, one binding | 4 | 99 |
| `decide_named`, needs approval, no grant, three bindings | 11 | 181 |
| `decide_named`, needs approval, its own grant, three bindings | 27 | 504 |
| `dispatch`, allowed, one binding | 4 | 99 |
| `dispatch`, granted, three bindings | 26 | 497 |
| `invocation_identity`, one binding / three bindings | 7 / 16 | 117 / 323 |
| `with_gate`, a registry of two capabilities | 18 | 833 |

Sizes on x86_64: `CapabilityRegistry<u8>` 48 bytes, `PolicyGate<u8>` 40, `PolicyDecision` 48, `Grant` 24, `Returns` 32,
`Capability` 120. What allocates is the registry path, not the gate: argument validation materialises each value's
canonical spelling (`ValueSpace::canonical` answers a `String`) and two vectors of addresses; the identity is built as a
string before it is compared; `decide_named` clones the gate's name. A decision that allocates nothing would need a
validation that checks membership without spelling the value, an identity streamed against the grant's bytes (the
bindings ordered without a vector, the number form written through `core::fmt::Write`), the gate's name borrowed, and a
denial's guidance borrowed or static. That is a successor's to build: it changes the validation every seam shares.
Phase 664 built it, and the table is at zero (D12).

**D11.8 — the version says the API broke.** `0.0.1-alpha.1` becomes `0.0.2-alpha.1`. Under Cargo's rules a
requirement on `0.0.1-alpha.1` admits a later `0.0.1` pre-release, so `0.0.1-alpha.2` would read as compatible; `0.0.2-*`
is outside it, so a consumer that depends as D2 says fails to resolve, loudly. `0.1.0-alpha.1` lost: it would start a
minor line nothing decided. `CHANGELOG.md` is new with this entry, and records the Core commit followed.

**D11.9 — what else the pin moved.** Core's Phase 568 added a `write` case to `skeletonDag` (writes on a DAG built from
held nodes, one refused as a content-id collision) and a vector to `skeletonMerge`. The crate already answered both; the
legs gained the reader and the count. The leg table of D8 gains `capability_gate.rs`.

## D12 — 2026-11-14 — the decision path allocates nothing once the registry is built: membership on the value, the identity compared as it is written, names and guidance shared

Phase 664 brings D11.7's table to zero. It changes nothing a gate decides, nothing a grant covers and no byte of the
identity: the 41 gate-family vectors and the 144 capability vectors pass unchanged on both profiles, and a differential
of the library before and after (the same random inputs to both; below) found no answer that differs.

**D12.1 — went red first.** `tests/capability_alloc.rs` is a counting global allocator in its own test binary, counting
on the measuring thread only (the harness's own threads allocate while a test runs), armed after the registry, the
arguments and the grant are built and disarmed before the answer is dropped. Its first row is a known allocation of 16
bytes, counted as one allocation of 16, so a zero is a measurement and not a counter that never ran. On the library as
D11 left it, the leg failed with D11.7's counts exactly, on both profiles, debug and release alike: `decide_named` 4, 11
and 27, `dispatch` 4 and 26, `invocation_identity` 7 and 16. It added three rows D11.7 did not measure: a dispatch parked
for approval (11, the gate's name copied into the refusal), and a denial, named and dispatched (8 each, the name and the
guidance copied). `run.ps1` runs it on the embedded profile; `cargo test --all-features` runs it on the hosted one.

**D12.2 — membership is decided on the value.** `ValueSpace::validate` no longer spells the value to test it: an
integer or a number is read and compared with its bounds (`read_int`, `read_float`, neither of which allocates), a
length is counted, a member compared, any string admitted. The one place a numeric value is read and spelled is a
private `respelled`, which `validate`, `canonical` and the identity all go through, so the three cannot disagree on what
lies in a space. Argument validation builds a refusal only when it answers one: the repeated-address check scans the
list for the first binding an earlier one binds (the head of `repeated_addrs`, not collected), the declared addresses
an `UnknownArg` names are listed only for that refusal, and a hole's space is read where it is held, not cloned
(`SigEntry::space` still answers an owned space for its callers). Every refusal names exactly what it named, with the
same bytes: the capability family's 60 validation vectors and the differential below say so. A tree argument's
membership is the one exception, kept: its document is read by the strict reader (`slot_kind_of`), which builds its
value tree and holds its nesting on the heap (D10). Lost: a second, non-allocating reader for this one use, which would
be a twin of the reader the Kani harnesses cover, outside those proofs, and would have to refuse a repeated member
without remembering the members it has seen. A tree is now read once per argument where it was read twice. The gate
family declares no tree hole and the table has no tree row; a host whose capabilities take trees allocates in
validation, in proportion to the tree.

**D12.3 — the identity is compared as it is written.** The identity is produced through `core::fmt::Write`
(`Capability::write_invocation_identity`), field by field: the id, then each binding's address and its value, each field
escaped and terminated as `canonical_fields` does it (the escape is now a writer, `hash::FieldBody`, and
`canonical_field` writes through it), the value's canonical spelling written by `core::fmt` (an integer by its
`Display`, a number by `number::write_finite_double`, the canonical number form laid out from a 32-byte stack buffer,
which `format_finite_double` now writes through), and every value but a number written as sent, which is its canonical
spelling where it lies in its space and its own where it does not, so a tree is never parsed here. The bindings are taken
in address order by selection, each pass the least binding after the one last written, equal addresses in list order
as the reference's stable sort keeps them: `O(n²)` comparisons of addresses and no vector. `Grant::covers_invocation`
compares the grant's bytes with the identity as it is written and stops at the first that differs; `judge` asks it, and
only when a gate answers `NeedsApproval`. `Grant::covers`, the comparison of two built identities, is unchanged.
`invocation_key` writes the same fields into its pre-image, so the key and the identity still share one spelling (D11.4)
and the identity's ordering and spelling are written once. Lost: sorting a stack array of indices, which bounds the
number of bindings, or falls back to a heap past the bound; and comparing a hash of the identity, which is a digest the
reference rejected (its D249.8). Certified against `invocationIdentity` and `grantCovers` byte for byte, on both profiles:
the leg answers both in the written forms and asserts each agrees with the built one, including
`gate-identity-out-of-address-order`, `gate-identity-integer-spelling`, `gate-identity-number-spelling`,
`gate-identity-holds-the-separator`, `gate-identity-holds-the-escape` and `gate-grant-own-other-spelling`. Nothing in
Core's shape forced an allocation: the identity as a canonical string, ordinal address order and the canonical number
form are each producible a byte at a time.

**D12.4 — names and guidance are shared, not copied.** A gate's name is `Arc<str>`, built once when the gate is built,
and `decide_named` answers it borrowed from the registry, `(&str, PolicyDecision)`, `""` where every gate allows. A
denial's guidance is `Arc<str>` and `Arc<[Arc<str>]>`, so a gate that builds its guidance when it is built hands every
denial it makes a reference count; `InvokeError::PolicyRefused` and `ApprovalRequired` carry the gate's own name and
guidance the same way. `Arc<str>` is the twin of the reference's immutable strings, and it costs an atomic increment
where a copy cost an allocation. `PolicyDecision::deny` with no alternatives allocates nothing (`Arc<[T]>`'s `Default`
shares one empty allocation, measured on rustc 1.97). Two alternatives lost. `Cow<'static, str>`: a literal costs
nothing, but a gate built from data at run time (the conformance leg builds every gate from the file's table, as a host
configured from a document would) copies its text into every denial, which is the copy this phase removes. A lifetime on
`InvokeError`, borrowing the gate: every seam and every caller of the error would carry it, for a gain `Arc` already
gives. Cost: `D11.3`'s atomics, already required; the text is built once when the gate is built, an allocation that
belongs to building (D12.6). A gate that builds its guidance inside its decision still allocates there, on each denial;
the type documents it.

**D12.5 — what `invocation_identity` answers.** On every profile it answers the identity as a `String`, which is one
allocation: the identity is written once to measure it and once into a string of exactly that capacity, so the row
measures one allocation of the identity's own length (20 bytes for the family's three-binding call, 9 for one binding),
asserted so. The crate requires `alloc` on every profile, so no profile lacks it; what the embedded profile is offered
in its place is `write_invocation_identity` into a buffer of the host's own (the leg writes it into 64 bytes of stack,
zero allocations), and `Grant::covers_invocation` for the decision itself. `grant` still mints a `Grant` holding the
identity: minting an approval is the host's act at approval time, not part of a decision.

**D12.6 — building a registry may allocate.** `register`, `with_gate`, `unregister` and `under` answer a new registry
(D11.3's persistence), cloning its capabilities and gates; a gate is an `Arc` of its function, and its name and guidance
are built with it. Measured: `with_gate` over the family's three capabilities, 27 allocations of 1,222 bytes (28 of
1,219 before: the name is an `Arc<str>` where it was a `String`). A device host builds its registry once, when it
starts, and decides with nothing allocated from then on, which is the claim the leg holds. Lost: building without a
heap, which is D12.8's question.

**D12.7 — the arguments stay owned pairs.** The decision path takes `&[(String, String)]`, as the gate's function does,
the twin of the reference's `(string * string) list`. A host that receives a command as text and splits it into
borrowed slices builds those strings before it calls, two allocations a binding, on its side of the call. Making the
argument type generic (`AsRef<str>` pairs) changes every signature the capability surface has, the gate's function
among them, and is a change of its own; it is not taken here, and it is the first thing a host with no allocation per
command would ask for.

**D12.8 — a host with no allocator at all.** After this phase what still needs a heap is the representation, not the
decision: a declaration's id, holes and spaces (`String`, `Vec`), the registry's two vectors, a gate's `Arc`, a grant's
identity, and the arguments (D12.7). Such a host would need declarations as static data (`&'static str`,
`&'static [SigEntry]`), a registry over a fixed array, gates as `&'static` functions, grants written into its own
buffers, and borrowed arguments: a second set of types beside every declaration type and the codec, which the reference
does not have. It is not this crate's to offer now. A device that has no general-purpose heap can give the crate a fixed
region as its global allocator for the build at start (the usual `no_std` arrangement), after which, by D12.1's leg,
deciding takes nothing from it. Should a host appear that cannot do even that, it is a phase of its own, escalated, not
assumed.

**D12.9 — costs, measured.** x86_64 (`x86_64-pc-windows-msvc`), release, the library before (`8546e32`) and after on the
same registry and arguments, in a scratch crate depending on both; time is a loop of 10⁶ calls around `black_box`, the
best of five; stack is the depth a call overwrites in a stack painted with a pattern beforehand (an empty call reads 64
bytes). Debug and release agree on every allocation count; the timings are release only.

| Call | Time before → after (ns) | Stack before → after (bytes) |
|---|---|---|
| `decide_named`, allowed, one binding | 139 → 24 | 2352 → 1504 |
| `decide_named`, needs approval, no grant, three bindings | 452 → 74 | 3728 → 2568 |
| `decide_named`, needs approval, its own grant, three bindings | 1095 → 239 | 3904 → 2568 |
| `dispatch`, granted, three bindings | 1066 → 234 | 4032 → 2792 |
| `validate_args`, three bindings | 384 → 55 | 2992 → 2024 |
| the grant check: `covers(&invocation_identity)` → `covers_invocation` | 628 → 159 | 3104 → 1464 |
| `invocation_identity` (`String`), three bindings | 604 → 315 | 3104 → 1752 |
| `invocation_key`, three bindings | 728 → 357 | 3072 → 2448 |
| `invocation_identity`, sixteen bindings out of order | 1866 → 3516 | 2304 → 1072 |
| `format_finite_double`, `1.5` / `1e300` / `0.30000000000000004` | 152 / 216 / 320 → 72 / 82 / 135 | 2160 / 2160 / 3008 → 1376 / 1376 / 2224 |

The selection order is the one cost that grows: at sixteen bindings the identity takes 1.9 times as long as the sort
did. In a decision it is bounded by the capability's holes, since validation refuses a repeated or an unknown address
before a gate is asked, and a declaration's holes are few. The stack a decision takes is the formatting machinery's
(`core::fmt` writing a number), about 2.5 KiB at most here, below what the allocating path took.

**D12.10 — certified.** The final table, every row zero on the embedded profile and on the hosted one, in debug and in
release, held by the gate: `decide_named` allowed, needing approval without a grant and with its own, and denied;
`dispatch` allowed, granted, granted with its bindings out of address order and its numbers respelled, parked, and
denied; the identity written to a stack buffer, one binding and three; `covers_invocation` with its own grant and with
another; `covers` given the identity; a denial of shared guidance with no alternatives. The differential (a scratch
crate, the two libraries side by side): 2,998,593 random finite doubles through `format_finite_double`, 400,000 random
invocations over two capabilities that between them hold every hole kind and every space (identity, key,
`validate_args`, `validate_args_all`, `canonical`, `validate`, and `covers_invocation` against the old identity, its
extension and its truncation), and 200,000 named decisions and dispatches under two gates and three contexts: no
answer differs, and the comparison was shown to fail when one side's arguments were cut.

**D12.11 — the version.** The public registry and seam types moved (D12.4's field types, `decide_named`'s answer,
`PolicyGate::new`'s name), so `0.0.2-alpha.1` becomes `0.0.3-alpha.1`, outside every requirement on `0.0.2-*`, by D11.8's
rule. The pin does not move: Core `c7d3013` (`0.43.0`, a draft, not released).

## D13 — 2026-11-16 — the port follows Core 0.43.0 as released: a document is a value space of its own, read by the strict reader and keyed by its canonical text, and the query registry's grant is not ported

Phase 665 moves `core-pin.json` from the `0.43.0` draft (`c7d3013`) to the commit Core released as `0.43.0`
(`a9592b7`, tagged `v0.43.0`). Between the two, Core added the document value space (its Phase 644, D260) with the
law family `capabilityDocumentLaws`, and the query registry's decision context and grant (its Phase 651, D261) with
the law family `queryGrantLaws`. No vector file of a family the crate certified at `c7d3013` moved.

**D13.1 — went red first.** At the new pin, before the crate changed, the capability leg gained the document family
on both profiles and every one of its 18 vectors failed: each declaration's hole space is `{"$type":"document",…}`,
which the declaration reader refused as `UnknownTag` at `signature.holes[0].space.$type`. Behind that fault sat three
the family needs: the pipeline seam's value-space table had no `document` row (three of the five type-check vectors
declare a document output), no membership or canonical text for a document argument, and no document arm in the
space relation. Every other family passed unchanged, hosted and embedded. `queryGrantLaws` had no leg to run red: the
crate has no query declaration to read its registry with (D13.5). A new leg, `tests/families.rs`, enumerates every
manifest under `conformance/` and fails on a family the crate neither answers nor names; it named `queryGrantLaws`,
and also `identity` (D13.6), which the crate had not answered at `c7d3013` either.

**D13.2 — the representation: a case of the value space, the type name carried and never read.** `ValueSpace` gains
`Document(Option<String>)`, the twin of the reference's `Document of typeConstraint: string option`, written
`{"$type":"document"}` with an optional `documentType`. The crate interprets the name nowhere: it is compared in the
relation and spelled in the description, as a `SlotTree`'s kind is. Two alternatives lost. A document as `AnyString`
is the reference's rejected (A) (D260.2): sound by membership, and it leaves a structured value untyped in the
relation and in a pipeline's edges. Marking `ValueSpace` `#[non_exhaustive]`, so that the case does not break an
exhaustive match, lost: the enum is the twin of a closed union of the reference, and a host that matches over it
should be told at compile time that a space it does not handle now exists. So this is a source break for a consumer
that matches exhaustively over `ValueSpace` (D13.9).

**D13.3 — membership is the strict reader, and no `null`.** A string lies in a document space exactly when the
strict reader (`json::parse`, the reader of D5 and D10, depth 512) reads it and the value it read holds no `null`:
the twin of `Space.documentOf`, whose reader refuses `null` and whose guarded writer refuses a repeated member, an
ill-formed string and a non-finite number. Those three are refused by the strict reader already (its rows 1, 6 and
7: a repeated member, an unpaired surrogate, an exponent that overflows or an integer past 2^53 that is not its
double's canonical layout), so what the crate adds is the `null` check, a walk over the value it read with an
explicit stack. The bare `NaN`, `Infinity` and `-Infinity` are not JSON (row 4), so a document holds no non-finite
number, as Core refuses it. The type name is never read, so every document space has the same members. An argument
that is no document is `ArgOutOfSpace` carrying the space, never `UninvocableArg` (D260.3): a document could fill the
hole, and the refusal's description says what would. Membership renders nothing: the canonical text is written only
where it is handed on (D13.4). Two alternatives lost. Reading through the decode layer's null-free model
(`decode::parse`), which refuses `null` at the reader as the reference does, then rendering that model: the crate has
no non-recursive writer for it, and the one copy of it into `JVal` it has (`canonical_form`'s) recurses, which D10
removed from every path an argument reaches. A second reader that decides membership without building the value, for
D12.2's reason; it would also have to remember every member name it has seen to refuse a repeated one, an allocation
of its own. What membership costs is measured in D13.8.

**D13.4 — the canonical text is the crate's to produce, as the reference decided.** A document's one spelling is its
cross-host canonical text (`render_canonical` of what was read: members sorted by UTF-16 code unit, no white space,
the canonical number form, every control character as `\u00xx`), the twin of `Space.canonical` handing on
`documentOf`'s text (D260.5: the space canonicalises). `ValueSpace::canonical` answers it, and the capture key and the
identity write it for a binding in a document hole that lies in its space, so two spellings of one document are one
key and one grant (`document-key-canonical` and `document-key-respelled` hold one key). Taking the text as given lost
to the reference's own ruling and to those two vectors: an argument reaches the registry from callers that are not an
encoder, in any member order. An argument outside its space is written as sent, as every other space's is (D12.3).

**D13.5 — `queryGrantLaws` is not adopted, and the reason is the crate's scope, not the contract.**
*Amended by D14 (2026-11-16): the operator ruled the query declaration, its admission gate and the read's identity
built (Phase 679), and the family is answered.* The family asks
for the identity of a read (`Query.invocationPreimage`) and whether a grant covers it, over four query declarations
it reads from their canonical text. The crate holds no query declaration, no query codec and no query registry. What
it calls its query seam is the refusal codec for the query seam's documents (`QueryError`, `ColumnPredicate`,
`SortKey`; D8, certified by `seamRefusals`), which reads and re-encodes those documents and holds no typed value.
Adopting the family therefore means porting the query declaration (its `params`, `resultSchema`, `effect`, `source`,
paging, and the `where` and `orderBy` members), the reader's admission gate (`QueryRegistry.admissionFaultAt`:
unknown columns, predicate type mismatches, inapplicable predicates, ill-formed literals, repeated sort columns) and
the identity, of which only the identity is what the 19 vectors check: every declaration they carry is one the gate
admits. Two alternatives lost. Porting the declaration and its gate here would add a public surface the vectors
certify at four admitted documents, its refusals certified by nothing, which is the partial answer under the
reference's name D11.6 refused for the pipeline check; and D8's rule is that a typed value is not offered until a
consumer needs one, and the crate's consumer, a device host, decides capabilities and makes no query reads. An
identity function over a query's id, the canonical text of its filter and its order, and the bindings, with no
declaration behind it, would answer the vectors and let a host mint a grant for a read the reference refuses to
declare. So the family is named by `tests/families.rs` and stays `proposed` for this crate. The decision context and
the grant the query registry gained have nothing in the crate to attach to; the capability registry's (D11) are
unchanged. Should a consumer need reads, porting the query declaration, its admission gate and the read's identity is
a phase of its own, certified then against this family and a refusal family for the declaration.

**D13.6 — the fate of every other family Core added or moved since `c7d3013`.** *`identity`'s fate is restated,
with the work it would take, in D14.7.* `capabilityDocumentLaws`: adopted
(D13.2 to D13.4). `capabilityLaws`, `capabilityGateLaws`, `seamRefusals`, `skeletonDag` and `skeletonMerge`: only
Core's adoption records for the crate moved (to `55c145f`); the vectors did not, and every one passes as before.
`conformance/idl/contracts.json` (`idlContracts`, Core Phase 644) is not answered: it certifies a reader of an IDL
vocabulary's contracts, its refusals, its projection onto a capability and the differ's classes, and the crate reads
no contract; its directory carries no manifest, so no adoption record could hold a count (D8's rule for the
`encoding/` table at the time), and `tests/families.rs` does not see it. `idl/idl.schema.json` gained the contracts
and is not read by the crate. `identity` (`identity/manifest.json`, Core Phase 578) was present at `c7d3013` already
and was neither answered nor named then; it is named now: a declared record's identity under its IDL vocabulary, over
a fixture vocabulary, which needs an IDL vocabulary reader the crate holds only in the slice the canonical-form family
reads (`canonical_form`). Adopting it is not this phase's: it is not a family Core added or moved since the last pin,
and it is reported so that it is no longer silent.

**D13.7 — the description and the relation.** `describe` answers "a document: well-formed JSON" and "a document of
type 't': well-formed JSON". In the relation the documents are a third family by the trees' rule: an unconstrained
document space admits every document space, a constrained one only the same name, and no scalar or tree space admits
a document or is admitted by one (`document-pipeline-output-*`). A pipeline's edge mismatch names the family
`document`. The pipeline seam's value-space table gains the row `document` (`src/seam.rs`), so a node's declared
output may be a document space; the seam's well-formedness rule already admits a space it has no bound rule for,
which is the reference's rule for a document.

**D13.8 — what a document argument costs, measured.** Allocations: `tests/capability_alloc.rs` gains a second leg
over a registry holding the family's document capability beside `read`, under a gate that parks the document for
approval; the counting allocator of D12.1, x86_64, on both profiles, debug and release, every count equal. Times: a
scratch crate depending on the library built as the embedded profile builds it (`default-features = false`), release,
x86_64 (`x86_64-pc-windows-msvc`, rustc 1.97.0), a loop of 2×10⁵ calls (2×10⁴ for the largest document) around
`black_box`, the best of five, with the answer consumed so the call is not elided; the times grow with the document,
which is the evidence the work was done.

| Call, the family's document (43 bytes, respelled) | Allocations | Bytes |
|---|---|---|
| `validate_args`, one document, either spelling | 11 | 858 |
| `decide_named` / `dispatch`, allowed, one document | 11 | 858 |
| `decide_named`, its own grant, the document respelled | 34 | 2,049 |
| `Grant::covers_invocation`, the document respelled | 23 | 1,191 |
| the identity written to a stack buffer, one document | 23 | 1,191 |
| `validate_args`, refused: no document (`order`) | 4 | 58 |
| `decide_named`, `read` (no document hole), on the same registry | 0 | 0 |

| Document | `validate_args` | `document_of` | `invocation_key` | `covers_invocation` |
|---|---|---|---|---|
| the family's, 43 bytes | 417 ns | 940 ns | 1,209 ns | 905 ns |
| an order of 10 lines, 592 bytes | 6.3 µs | 11.6 µs | 12.8 µs | 11.5 µs |
| an order of 100 lines, 5,722 bytes | 59.8 µs | 104 µs | 112 µs | 104 µs |

An `anyString` hole given the same strings validates in 9 ns. So membership costs about 10 ns a byte here and the
canonical text, which the key, the identity and a grant's coverage write, about 18 to 20 ns a byte; both are linear in
the document. What forces the allocations is the reading: the strict reader builds the value (its strings, its arrays
and its member lists), keeps its nesting stack on the heap (D10) and remembers each object's member names to refuse a
repeated one (row 1); the `null` walk keeps its own stack; and the canonical text is a string, with each object's
members collected to sort them. None of it is a copy the decision path could drop: a document's membership is a parse,
and its one spelling is a rendering, by Core's own definition. A refused argument allocates for its refusal (the
space and the value it names, as every `ArgOutOfSpace` does). The stack a document read takes does not grow with its
depth, because neither the reader nor the writer recurses (D10). So D12's claim stands as stated, for the decision
path of a capability with no tree or document hole: a host whose capabilities take documents allocates in validation
and in a grant's check, in proportion to the document, as a host whose capabilities take trees already did (D12.2).
Lost: holding a document hole's canonical text from validation to the identity, which would save the second read on
a granted decision at the price of carrying a cache through every seam signature; the identity is asked only when a
gate answers `NeedsApproval`.

**D13.9 — the version.** `ValueSpace` gains a case, which breaks every exhaustive match over it (D13.2), so
`0.0.3-alpha.1` becomes `0.0.4-alpha.1`, outside every requirement on `0.0.3-*`, by D11.8's rule. Nothing else in the
public surface moved: `document_of` is added. The pin is Core `a9592b7` (`0.43.0`, released).

**D13.10 — certified.** At Core `a9592b7`, on the hosted profile and again without `std`: `capabilityDocumentLaws` 18
of 18 (validation 9, the key 2, the declaration round trip 2, the pipeline type check 5), `capabilityLaws` 144 and
`capabilityGateLaws` 41 unchanged, and the zero-allocation table of D12.10 at zero for every row it had, debug and
release. On the hosted profile, every other family the crate answered at `c7d3013`, with the counts its manifests
declare: none of their vector files moved. Port-side tests hold what the vectors do not reach: a number no finite
double spells (`1e400`, `-1e400`), the bare `NaN` and `Infinity`, a `null` below the root and an unpaired surrogate
are each no document; the canonical text sorts by UTF-16 code unit and collapses a negative zero; the codec reads the
type name as a string; the relation's nine document cases; and a number space refuses `NaN`, `Infinity`, `-Infinity`,
`1e400` and `-1e400` as an argument (D260.7).

## D14 — 2026-11-16 — the crate holds the query declaration, its codec and admission gate, the read's identity, and a query registry whose grant check is decided by the capability registry's own function

Phase 679 ports what D13.5 named: the query declaration (the reference's `Query`, its `QueryParam`, `ColumnPredicate`
and `SortKey`), its declaration reader and writer, the admission gate the registry and the reader share
(`QueryRegistry.admissionFaultAt`), the identity of a read and the grant bound to it (D261.3), and a query registry
that decides a read under a decision context and a grant through the function the capability registry decides by
(D261.7). Certified against `queryGrantLaws` at Core `a9592b7` (`0.43.0`, released); the pin does not move.

**D14.1 — the operator ruled that this is built, and D8's consumer rule is amended for it.** D8 offers no typed value
per seam union until a consumer needs one, and D13.5 declined the family on that rule: the crate's one consumer, a
device host, decides capabilities and makes no query reads. The operator has ruled that the query declaration, its
admission gate and the read's identity are built regardless (Phase 679), so that a host which reads, or which must
hold a read's grant, has the reference's contract rather than a lookalike. That ruling is recorded here and not argued
again. What D13.5 refused stays refused: an identity with no declaration behind it, and a declaration whose refusals
nothing certifies. So the whole admission gate is ported, and each refusal the reference's vectors do not reach is
pinned by a port-side test to the reference's text (D14.4).

**D14.2 — the representation: a module of its own, the reference's records as plain data.** `src/query.rs` holds
`QueryParam { name, ty, required }`, `ColumnPredicate` (eight cases: the five comparisons carrying a column and a
`Cell`, `Contains` a column and a text, `IsNull` and `IsNotNull` a column), `SortDirection`, `SortKey { column,
direction }` and `Query { id, params, result_schema, effect, source, timeout_ms, page_size, filter, order_by }`, the
twins of the reference's types, built from the crate's own column types (`ColumnType`, `Cell`, `Field`, `DataSource`)
and the capability's `EffectClass`. The paging and time budget are `Option<i32>`, the reference's `int option`, read
by `Decoder.int`. The wire member `where` is the field `filter`, because `where` is a Rust keyword. `QueryError` is the
whole of the reference's closed union (twenty cases), so a host's resolver refuses in the seam's own vocabulary.
Fields are public, as `Capability`'s are: a declaration a host builds by hand is admitted or refused at `register`, as
the reference's record is. Five alternatives lost. Putting the declaration in `src/seam.rs`: that module is the
declaration-driven codec of the seams' documents and holds no typed value; a typed declaration, its gate and a
registry are a layer above it, as `capability` is. `r#where` as the field name: legal, and every reader would write
`q.r#where`. A comparison carrying its column type beside the literal: the reference holds the `Cell` and derives the
type (`Cell.typeOf`), and a second copy of the type could disagree with the cell. A predicate held as the seam reader's
re-encoded document, untyped: admission and the identity read the cell, and a host matching on a predicate would have
to parse it. A declaration validated at construction and opaque: the reference's record is data and its gate is the
registry's and the reader's, and a refusing constructor would be a third place the gate runs.

**D14.3 — the codec: the lenient reader, through the seams' one reader for the filter and the order.**
`Query::decode` reads a declaration as `QueryCodec.decodeDetailedWith ReadPolicy.Lenient` does, in its order: `$type`
(`query`), `id`, `params` (each `name`, `type` by its tag, `required`), `resultSchema` (each a field the column codec
reads, else `OutOfRange` at its index), `effect` (the capability codec's reader), `timeoutMs`, `pageSize`, `where`,
`orderBy`, `source` (any JSON, then the column codec, whose refusal is `OutOfRange` at `source`), and last the admission
gate, whose refusal is `OutOfRange` at the member at fault with the registry's sentence. An unknown member is read
past. Each `where` item and `orderBy` key is read by the seam module's declaration-driven reader (`seam::read_at` over
`COLUMN_PREDICATE` and `SORT_KEY`: D8's one reader, certified by `seamRefusals`), and the typed value is a total
projection of what that reader answers, so a predicate's refusals are that reader's, on code and path, and no second
predicate reader exists. `Query::encode` writes the canonical text of `QueryCodec.encode`: `timeoutMs`, `pageSize`,
`where` and `orderBy` omitted when absent or empty, and a comparison's literal written as the column codec writes a
cell of the cell's own type (`QueryWire.literalJson`, with its fallbacks for a literal its type cannot hold, so the
writer is total). Not ported, with no vector and no consumer: the strict reading (the capability declaration's port
reads leniently only, D11), the result page and its `Deferred` envelope, `decodeArgs`, `toJsonSchema`, and a typed
writer of `QueryError` (the seam module reads and writes its document). Two alternatives lost: a hand-written predicate
reader, which D8 rejected for every seam union; and the strict reading now, a surface no vector certifies.

**D14.4 — the admission gate, whole.** `admission_fault_at` is the twin of `QueryRegistry.admissionFaultAt`, and
`register` and the reader both run it, so a declaration that decodes is one `register` admits. In order: a parameter
named twice (`DuplicateParam`, the first repeat, at `params`); then each `where` predicate in turn (at `where.i`): a
column the result does not declare (`UnknownColumn`, naming the result's columns), `contains` on a column that is not
a string (`PredicateNotApplicable`), a `Null` literal (`IllFormedLiteral`, "a null literal compares with nothing; test
for absence with isNull"), a literal of another type (`PredicateTypeMismatch`; a timestamp literal fits any timestamp
column at least as fine as its own unit, Phase 422), and a literal its column cannot carry (`IllFormedLiteral`, with the
column layer's reason: `Column::of_cells`, then `Table::validate`); then each `orderBy` key (at `orderBy.i`): an
undeclared column (`UnknownColumn`), then a column named before (`DuplicateSortColumn`, at the second). `register`
checks the id first (`DuplicateQuery`). Each refusal answers `QueryError::describe`, the twin of the reference's
sentence. Core emits no vector for any of these refusals (no family reads a query declaration): the port's tests pin
each one to the reference's text at the pin, by file and line, and the envelope asks Core for vectors.

**D14.5 — the read's identity, field by field, as D261.3 fixes it.** `Query::invocation_identity(args)` is the twin of
`Query.invocationPreimage`: the canonical pre-image (`hash::canonical_fields`, no hash) of the query id; then, for a
declaration with a filter, the fields `""`, `w` and the canonical text of its `where` array, and for one with an order
`""`, `o` and the canonical text of its `orderBy` array, filter first; then each binding in name order (ordinal, equal
names in list order, as the reference's stable sort keeps them), three fields each: its name, its cell's tag (`i` int,
`f` float, `b` bool, `s` string, `d` date, `t` timestamp, `m` decimal, `n` null) and its payload (an integer's decimal
digits; a float in the canonical number form, a negative zero `0`, a non-finite one its quoted token, as
`FloatLayout.canonical`; `1` or `0`; a string, date, timestamp or decimal as it stands, never canonicalised; empty for a
null). A cell is identified by its tag, not by its parameter's type, so an integer bound to a float parameter and the
float of the same value are two reads. No page token is in it (D261.4). `Query::grant` mints the grant for exactly the
read, and `Grant::covers_read` compares a grant with the identity as it is written, stopping at the first byte that
differs (D12.3's form) and building nothing. Three alternatives lost, each the reference's own: the capture key (a
32-bit hash, D249.3); a cell canonicalised by its parameter's type (D261.3); the page token in the identity (D261.4).
The capture keys (`invocationKey`, `invocationKeyPage`) are not ported: nothing in the crate captures a read, and the
vectors that present one as a grant name it as text.

**D14.6 — one function decides at both registries.** `PolicyGate<C, D = Capability, A = [(String, String)]>` is the
gate over a declaration `D` and its argument list `A`, the twin of the reference's `PolicyGate<'Ctx, 'Decl, 'Args>`;
at its defaults it is the capability gate it was. The gates' join with the grant's lift, the twin of
`RegistryPolicy.decideGranted`, is one crate function, `decide_granted`, which `CapabilityRegistry` and the new
`QueryRegistry<C = ()>` both call: the first gate of the most refusing rank decides, and a `NeedsApproval` is lifted to
`Allow` when the registry's coverage check answers that the grant is for exactly this invocation, a check asked only
then. The query registry is the capability registry's twin in every verb it carries (`new`, `empty`, `register`,
`try_find`, `enumerate`, `ids`, `unregister`, `with_gate`, `gates`, `resolve`, `decide_named`, `dispatch`, `under`):
resolve (`NoSuchQuery` naming the held ids), validate (`Query::validate_params`, the twin of `Query.validateParams`: a
repeated name, then an unknown name or a cell that does not widen to its parameter's type, then required names left
out, then required names bound only to `Null`), then decide; a dispatch refuses `QueryPolicyRefused` or
`QueryApprovalRequired` from that decision and runs the host's resolver exactly once on `Allow`
(`FnOnce(&Query) -> Result<T, QueryError>`, D11.5's performer). The registry renders each query's filter and order
text once, when it registers it, and its grant check writes the identity from that text, so deciding a read allocates
nothing once the registry is built, for a read with a filter as for one without (D14.8). Not ported, with no vector
and no consumer: the denial observers (D11.3's reason), `decide` (the decision folded into a `Deny`), the paging,
captured and replayed dispatch verbs, `Query.invoke` with its `Deferred` envelope and `ResolveFault`,
`validateParamsAll`, and `replace`, `restrict` and `union`. Three alternatives lost. A generic seam type both
registries wrap (the reference's `Seam`): it would restructure the capability registry and its D12 table for nothing
the decision needs, since the decision is the one function and the rest is each registry's own lookup and validation.
A trait the declarations implement: the same sharing with more ceremony. A second gate type and join for reads: two
functions would decide, which D255.6 and D261.7 forbid.

**D14.7 — the `identity` family stays named, not adopted, and should be a phase of its own.** `identity`
(`conformance/identity/`, Core Phase 578) certifies a declared record's identity: the record read through its IDL
vocabulary, its canonical form under it (the vocabulary's key order, absent optionals omitted, a member at its default
not written), rendered under a named profile (`v1` or `v2`) and hashed. The crate cannot answer it honestly. The
vocabulary is a test fixture of the reference (`tests/Fuaran.Core.Tests.Fixtures/IdentityIdl.fs`), not conformance
data, so a port would restate it by hand from prose, and nothing would certify that the restatement is the vocabulary
the lines were computed under. Two of its nine lines are rendered under `v1`, whose short escapes for line feed,
carriage return and tab are the profile D8 declined; writing them needs a second escape mode beside the one
`src/json/escape.rs` proves. And it needs an IDL vocabulary reader and writer (a tagged root, key order, `Optional`,
`OmitDefault`), of which the crate holds only the canonical-form slice (`canonical_form`). The work: a vocabulary
interpreter for a record's canonical form, the `v1` rendering as a writer of its own (outside the proved escape, or with
its own proof), and the fixture vocabulary emitted by Core as data beside the table, which Core should be asked for. It
should be a phase of its own, taken when a consumer needs to mint a record's identity; it is not this phase's, which is
the read's. `tests/families.rs` names it with that reason, and still fails on any family Core emits that the crate
neither answers nor names.

**D14.8 — what it costs on the embedded profile, measured.** Allocations: `tests/capability_alloc.rs` gains a third
leg over a query registry of `queryGrantLaws`' `ledger` (no filter, no order) and `shaped` (both) under a `u8` context,
the counting allocator of D12.1, on both profiles, debug and release, every count equal. Asserted at zero:
`validate_params`; `decide_named` allowed, parked with no grant, with its own grant (for `shaped` too, whose filter
and order the registry rendered when it registered it), and with another read's grant; `dispatch` granted (`shaped`)
and parked; the identity written to a stack buffer and `Grant::covers_read`, for `ledger`. Reported, not asserted:
asked of a declaration alone, a filtered and ordered read's identity and coverage render that text, 56 allocations of
1,858 bytes for `shaped` (the predicate and key documents built, then written). Every row of D12's table stays at zero.
Times: a scratch crate depending on the library built as the embedded profile builds it (`default-features = false`),
release, x86_64 (`x86_64-pc-windows-msvc`, rustc 1.97.0), a loop of 2×10⁵ calls (2×10⁴ for the codec and `register`)
around `black_box`, the best of five, the answer consumed; run twice, every row within 5% of the other run. The control
is D12.9's capability row on the same machine, 225 ns against D12.9's 239.

| Call | Time (ns) |
|---|---|
| `Query::decode`, `ledger` (267 bytes) / `shaped` (436 bytes) | 4,641 / 10,061 |
| `Query::encode`, `shaped` | 4,638 |
| `admission_fault`, `shaped` | 284 |
| `register`, `shaped`, into an empty registry | 2,672 |
| `validate_params`, one binding / seven bindings out of name order | 9 / 115 |
| `invocation_identity` (`String`), `ledger` one binding / `shaped` one binding / `every` seven bindings | 74 / 2,172 / 661 |
| `Grant::covers_read`, `ledger` / `shaped` (renders its filter and order) | 33 / 2,151 |
| `decide_named`, `ledger`: allowed / parked, no grant / its own grant | 30 / 31 / 66 |
| `decide_named`, its own grant: `shaped` / `every`, seven bindings | 176 / 508 |
| `dispatch`, granted, `shaped` | 180 |
| control: capability `decide_named`, its own grant, three bindings | 225 |

So the registry's rendering at `register` is what keeps a filtered read's decision at 176 ns where asking the
declaration alone takes about 2.2 µs: the filter and the order are rendered once per query, not once per decision.
The declaration codec costs what reading and writing JSON costs (about 20 ns a byte to read, D10's reader) and runs
when a declaration arrives, not when a read is decided.

**D14.9 — the version: the gate's constructors became generic, which breaks inference in one shape of caller.**
`PolicyGate` gains the type parameters `D` and `A` with the capability's types as their defaults, so every place that
names `PolicyGate<C>` means what it meant. But `PolicyGate::new` and `ignoring_context` now take a closure over `&D`
and `&A` for any `D` and `A`, and a closure whose declaration parameter is not annotated, built where no expected type
fixes it (`let g = PolicyGate::new("p", |_: &u8, c, _| … c.id …)` before `with_gate(g)`), no longer infers: `E0282`.
That compiles against `0.0.4-alpha.1` (`03bdc2d`) and not against this commit, measured with the same file against
both. So the change is breaking, and `0.0.4-alpha.1` becomes `0.0.5-alpha.1`, outside every requirement on `0.0.4-*`,
by D11.8's rule. Everything else is added: the module `query`, `Grant::covers_read`, and `PolicyGate`'s two type
parameters at their defaults. A gate passed straight to `with_gate`, or built where its type is declared (every gate in
the crate's own tests), is unchanged. Lost: keeping `new` and `ignoring_context` on the capability's instantiation and
naming the read's constructors apart, which avoids the break at the price of two names for one constructor of one type;
the reference has one, and the crate is a pre-release alpha whose version exists to say this. The pin does not move:
Core `a9592b7` (`0.43.0`, released).

**D14.10 — certified.** At Core `a9592b7`, on the hosted profile and again without `std`: `queryGrantLaws` 19 of 19
(`invocationIdentity` 11, `grantCovers` 8), each declaration of the file's head read through the declaration reader
and written back to the bytes it was read from, every identity answered in its written form and asserted equal to the
built one, and every coverage vector also decided by the query registry under a gate that asks for an approval;
`capabilityLaws` 144, `capabilityDocumentLaws` 18 and `capabilityGateLaws` 41 unchanged; D12's zero-allocation table
at zero for every row it had, and the read's rows of D14.8 at zero, debug and release. On the hosted profile every
other family the crate answered before, with the counts its manifests declare. `tests/query.rs` holds what no vector
reaches, each test citing the reference's file and line: every refusal of the admission gate and of the declaration
reader, the lenient read, the canonical text and its round trip, the `seamRefusals` predicate and order-key vectors
read inside a declaration, validation's refusals in order, the float payload's layout, the filter and the order in
the identity, and the registry's decision, dispatch, `under` and `unregister`. `tests/families.rs` names `identity`
(D14.7) and `encodingProfile` (D8) as the two families the crate does not answer.

## D15 — 2026-11-17 — the declared minimum stays 1.85, and the gate runs at it: every leg that compiles and tests, while the format check and the lints are the newest tools'

Phase 681. `Cargo.toml` has declared `rust-version = "1.85"` since D2, and nothing ran the crate at it: CI installed
stable and `run.ps1` used whatever the machine's default was. The declaration was a claim nothing checked. This entry
records what the crate does at 1.85, what the gate now runs there, and what it does not. No source file, no public
item and nothing the crate certifies changed.

**D15.1 — measured first, on the crate as it stood.** Two commits that changed the workflow only ran the whole gate
in CI at the minimum read from the manifest (rustup resolves `1.85` to `rustc 1.85.1 (4eb161250 2025-03-15)`) beside
stable (`rustc 1.100.0 (0ef1bc386 2026-11-09)`), on the crate at `f1dd15e`. At 1.85.1: `cargo fmt --check` passes;
`cargo build --all-features` passes with no warning; `cargo test --all-features` passes in every binary (the
conformance legs among them, against Core `a9592b7`); the premises in release, the capability and query legs without
`std`, and the `thumbv7em-none-eabihf` build pass; the allocation table passes in debug and in release, hosted and
without `std`. One leg fails: `cargo clippy --all-features --all-targets -- -D warnings` raises four lints the clippy
of 1.100.0 does not raise on the same tree, `clippy::manual_flatten` at `src/capability.rs:1279` and
`clippy::needless_lifetimes` at `src/capability.rs:1349`, `src/json.rs:317` and `src/seam.rs:643`. Nothing fails to
compile and no answer differs.

**D15.2 — the choice: keep 1.85; the format check and the lints run on stable only; every other leg runs at both.**
A lint is the tool's opinion of the source, not a property of the library a host compiles: what 1.85 must answer is
whether the crate builds there and answers every vector, every premise and every row of the allocation table as it
does at stable, and it does. The format check and the lints run where the newest tools are, which is the stable job
and a developer's default toolchain. Two alternatives lost:

- *Fixing the four lints so clippy runs at both.* It would touch three source files, `src/json.rs` among them, whose
  reader the Kani harnesses cover (`verification/README.md`), to satisfy a clippy that stable's no longer agrees
  with. The two clippys disagree today in one direction; nothing stops them disagreeing in the other as lints are
  added and retired, and a gate that runs two lint sets has to hold the source to the stricter of both at every
  moment. The lints stay one tool's, at stable.
- *Raising the minimum.* Edition 2024 needs 1.85, so 1.85 is the floor of what the manifest can declare; the crate
  passes there on every leg that is about the library, so a raise would leave out every host on 1.85 up to the raised
  version and buy nothing. No host is left out.

**D15.3 — the minimum is read from the manifest, in both places.** CI's minimum job reads `rust-version` from
`Cargo.toml` and installs that toolchain; `run.ps1` reads the same line. Neither names a version of its own, so moving
the declaration moves both, and they cannot drift apart.

**D15.4 — the local gate: it runs the minimum's legs when the toolchain is there, and names it missing when it is
not.** With no arguments `run.ps1` runs the format check and the lints, the legs on the default toolchain, and then
the same legs (build, tests, premises in release, the legs without `std`, the allocation table, the embedded build)
again through rustup at the minimum (`cargo +<toolchain>`), the toolchain found among those `rustup toolchain list`
names (`1.85`, or a patch of it). Nothing is installed from the script: it calls a toolchain only by a name rustup
lists as installed, with `RUSTUP_AUTO_INSTALL=0`. When the minimum is not installed it prints
`MISSING: the minimum Rust version Cargo.toml declares (1.85) is not installed, so its legs did not run.` with the two
commands that install it and its embedded target, and ends `fuaran-core-rs: green (minimum 1.85: MISSING, its legs did
not run)`: the run is green, and its last line says what it did not check. Every stage seeds `$LASTEXITCODE`, and the
missing branch sets it, so a skipped stage never reads another's exit code. `-MinimumOnly` runs the minimum's legs
alone and FAILS when the toolchain or its embedded target is missing; `-SkipMinimum` leaves them out by name. Lost:
failing by default when the minimum is missing, with an opt-out, as a missing embedded target fails. The gate is
`verify.ps1`, which passes no argument and runs on machines that hold one toolchain; failing there by default would
make every such machine red for a leg CI runs anyway, or teach everyone to pass the opt-out. The evidence that the
minimum's legs run is CI's minimum job, which calls `-MinimumOnly`, so a green minimum job is one whose legs ran.

**D15.5 — CI: one matrix, two jobs.** `gate (stable)` installs stable with rustfmt and clippy and runs
`run.ps1 -SkipMinimum`; `gate (minimum)` installs the manifest's minimum with its embedded target, no rustfmt and no
clippy, and runs `run.ps1 -MinimumOnly`. The Core checkout at the pin, the side-by-side layout and the actions are
unchanged.

**D15.6 — the allocation table, in debug and in release, at both.** D12.10 says the table is held at zero "in debug
and in release, held by the gate", but `run.ps1` ran `capability_alloc` in debug only (in `cargo test --all-features`
and in the leg without `std`): release was measured when D12 was written and not held after. The gate now runs it
without `std` in debug, and hosted and without `std` in release, each with `--nocapture` so the log carries every row,
on the default toolchain and at the minimum; the hosted debug run stays in `cargo test --all-features`. At 1.85.1 every
row the table asserts at zero is zero in all four: the fifteen rows of a capability's decision path and the ten of a
read's. The rows the table reports without asserting zero read the same at 1.85.1 as at stable (building a registry 27
allocations of 1,222 bytes, a document argument 11 of 858, a filtered read's identity asked of the declaration alone 56
of 1,858). No row is relaxed or made conditional on the compiler. The leg without `std` is moved out of the combined
capability and query run into its own stage, so it is not run twice.

**D15.7 — the version does not move.** The declared minimum did not rise and no public item changed, so by D11.8's
rule `0.0.5-alpha.1` stays. The pin does not move: Core `a9592b7` (`0.43.0`, released).

## D16 — 2026-11-17 — the minimum is the exact version the manifest declares: `1.85` is 1.85.0, and the minimum job fails unless the compiler that ran its legs reports it

Phase 682. D15 put the gate at the declared minimum, but CI installed the toolchain by the bare name `1.85`, which rustup
resolves to the newest patch of that release, and D15.1 measured 1.85.1. A host on 1.85.0 is inside what
`rust-version = "1.85"` admits and was outside what anything checked. This entry amends D15 where the minimum is read,
installed, found and reported; D15's text is history and stands as written. D15.2 stands: the format check and the lints
run at stable only. No source file, no public item and nothing the crate certifies changed.

**D16.1 — measured first, at 1.85.0, on the crate as it stood.** One commit that changed the workflow only (`09f21c8`:
the minimum job installs `1.85.0`) ran the gate in CI on the crate as it stood on `main` (`31bf80c`), against Core `a9592b7`. The
minimum job's compiler is `rustc 1.85.0 (4d91de4e4 2025-02-17)`, in the setup step and in `run.ps1`'s own
`rustc --version` stage; stable's is `rustc 1.100.0 (0ef1bc386 2026-11-09)`. At 1.85.0 every leg D15 runs at the
minimum passes: `cargo build --all-features` with no warning; `cargo test --all-features` in every binary, the
conformance legs among them; the premises in release; the capability and query legs without `std`; the allocation
table without `std` in debug and hosted and without `std` in release, every asserted row at zero and the reported rows
as D15.6 records them (27 allocations of 1,222 bytes, 11 of 858, 56 of 1,858); the `thumbv7em-none-eabihf` build. So
the job stays at 1.85.0 and the declared minimum stays `1.85`: nothing is fixed and nothing is raised.

**D16.2 — the floor is exact, and read by one rule.** A declared version with two components is read with `.0` as its
patch (`1.85` is 1.85.0, which is what Cargo admits under it); one with three is taken as written; anything else is
refused. The workflow reads `Cargo.toml` and applies the rule in one step, and installs that version; `run.ps1` reads
the same line by the same rule. The two readers cannot drift silently: if the workflow installs anything but the
version `run.ps1` derives, the minimum job is red (D16.3 shows it). Two alternatives lost:

- *Declaring `rust-version = "1.85.0"`.* It says the same thing to Cargo, and the rule is needed whatever the line says,
  since a two-component declaration must still mean its `.0`. The manifest's line stays.
- *One reader: the workflow asking `run.ps1` for the floor.* It would add a switch to the gate script that only CI
  calls, to remove a drift the job already turns red on.

**D16.3 — the minimum job asserts the version it ran at.** `run.ps1` lists the installed toolchains of the floor's
release (`1.85`, `1.85.0`, `1.85.1`, ...), asks each one's own compiler (`rustup which rustc --toolchain`) for its
version, and runs the minimum's legs on the toolchain whose compiler reports exactly the floor, chosen by what it
prints and never by its name. Cargo is handed that compiler (`RUSTC`) for the legs, so a caller's `RUSTC` or
`build.rustc` cannot put another compiler under the floor's name; the caller's `RUSTC` is restored after. Every stage at
the minimum is labelled with the floor and the toolchain, and the last line says `ran at 1.85.0 on <toolchain>`.
`-MinimumOnly` (CI's minimum job) fails before any leg unless such a toolchain is installed:
`FAILED: -MinimumOnly runs only at exactly 1.85.0, and no installed toolchain's compiler reports it`. Shown on the pull
request: a commit that pointed the job at `1.85.1` (`17159c7`) was red at the gate step with that line, and the commit
that removed it (`abeda89`) was green at 1.85.0. Lost: asserting in the workflow, comparing `rustc --version` after the install. It checks the
toolchain the step installed, not the one the legs ran on, and a local run would not carry it; the assertion sits
where the legs' compiler is chosen.

A toolchain is looked for only under a name that begins with the release (`1.85-…`, `1.85.0-…`); a toolchain linked
under another name, or `stable` on a day it is the floor, is not looked at, and the run names the floor as missing.

**D16.4 — the local gate, in each of the three cases.** With no arguments `run.ps1` still ends green whatever the
minimum's state, and its last line says which state it was (D15.4's reason holds: `verify.ps1` runs on machines that
hold one toolchain, and CI's minimum job is the evidence).

- *The floor installed:* the legs run there, as in CI (`fuaran-core-rs: green (minimum 1.85.0: ran at 1.85.0 on …)`).
  Exercised in CI.
- *Nothing of the release installed:* `MISSING: the minimum Rust version Cargo.toml declares, exactly 1.85.0, is not
  installed.`, with `rustup toolchain install 1.85.0 --profile minimal` and the target's command, and the last line
  `minimum 1.85.0: MISSING, its legs did not run`. Exercised on the landing machine, which holds stable only.
- *Another patch of the release, not the floor:* the same `MISSING` line, naming the toolchain and what it reports;
  then the legs run on that patch, every stage labelled `(1.85.1, NOT the minimum 1.85.0, …)`, and the last line
  `minimum 1.85.0: MISSING; its legs ran at 1.85.1 on …, another patch, not the minimum`. Exercised in CI, by a probe
  step in the same red commit. Lost: naming the floor missing and running nothing there. A patch release of the same
  minor compiles what the floor compiles with rare exceptions, so its legs are evidence about the release that costs
  nothing to collect on a machine that holds it (one that followed D15's remedy, `rustup toolchain install 1.85`,
  holds 1.85.1); the claim is never made from it, since the run leads with `MISSING` and never calls it the floor.

Under `-MinimumOnly` a later patch is never run: the job fails before any leg.

**D16.5 — what this amends in D15.** D15.1's measurement is of 1.85.1, as recorded; 1.85.0 is D16.1's. D15.3: the
minimum is read in both places by D16.2's rule. D15.4: the toolchain is found by the version its compiler reports
(D16.3), not by a name matching `1.85` or a patch of it, and the remedy names `1.85.0`. D15.5: the minimum job installs
the exact floor. D15.2, D15.6 and D15.7 stand.

**D16.6 — the version does not move.** The declared minimum did not rise and no public item changed, so by D11.8's rule
`0.0.5-alpha.1` stays. The pin does not move: Core `a9592b7` (`0.43.0`, released).
