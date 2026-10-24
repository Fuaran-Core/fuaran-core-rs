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
| `capability.rs` | `capabilityLaws` | `capability`, `hash` |
| `seams.rs` | `seamRefusals` | `seam` |
| `premises.rs` | `premiseStringModel`, `premiseFloatDigits`, `premiseInt53`, `premiseSha256`, `premiseRecursionDepth` | `json`, `decode`, `number`, `sha256` |

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
