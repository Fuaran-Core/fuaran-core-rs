# fuaran-core-rs

The Fuaran Core spine in Rust: canonical JSON with its proved grammar and parse limits,
the number form, SHA-256 content addresses, tree and op apply, the op-stream chain, the
DAG with its reconcile and diff, and the columnar, capability, propagation and seam
contracts. It answers the conformance families the .NET reference emits with a manifest, but for those `tests/families.rs` names (DECISIONS.md D8, D13), and certifies against the conformance vectors the .NET reference
([fuaran-core](https://github.com/Fuaran-Core/fuaran-core)) emits.

The crate is `no_std` + `alloc` by default, so it builds for microcontroller targets;
`std` is a feature.

## What it holds

| Module | What |
|---|---|
| `json` | the value model, the strict reader (one RFC 8259 text; a repeated member, trailing content, a number outside the grammar, a raw control character and an unpaired surrogate refused) with its limits record enforced on the way down, and the canonical writer |
| `number`, `decimal` | the canonical number form, and the exact decimal |
| `sha256`, `address` | the digest, and the `sha256:` content address |
| `decode` | the decode error contract — every refusal a code at a path — and the reading combinators |
| `tree`, `ops` | the skeleton tree and the apply of its six ops |
| `chain` | the op-stream chain over opaque payloads, and the actor |
| `dag` | the content-addressed op-DAG: writes, heads, orders, ranks, replays and the integrity walk |
| `merge`, `diff` | the common base and the N-lane reconcile under the ops' footprint, and the content-aware tree diff with its classification |
| `hash` | the FNV-1a fingerprint over UTF-16 code units, and the injective field encoding |
| `column`, `field`, `unit` | the columnar `DataSource` codec, its refusals and the pinned aggregates; the schema field, its diff, fingerprint and delta; the unit of measure's parser and canonical text |
| `propagation` | the evaluation order and its cycles, the dirty set of a change, and the incremental evaluation |
| `capability`, `seam` | the invocable-capability contract (the value spaces, a JSON document among them; argument validation, the invocation key, the declaration codec with its declared result, the registry, its gates under a decision context of the host's type, the grant bound to the invocation's identity, the pipeline type check); the seams' refusal and declaration documents, read by one declaration-driven codec |
| `query` | the query declaration (its parameters, result columns, effect, source, paging, filter and order) with its codec and admission gate, the identity of a read and the grant bound to it, and the query registry, whose reads are decided by the capability registry's own decision function |
| `canonical_form`, `profile` | an IDL vocabulary's canonical form (aliases, presence, defaults); the wire profile `<name>@<major>.<minor>` |

## Conformance

`tests/` certifies the crate against the vectors the reference emits, one file per group of families, read from a
checkout of [fuaran-core](https://github.com/Fuaran-Core/fuaran-core) at the commit `core-pin.json`
names: clone it beside this repository as `fuaran-core/` (or set `FUARAN_CORE_DIR`). The legs fail
when no checkout is found. Which families are run, with each leg's count, is
[DECISIONS.md](DECISIONS.md) D8 (D6 recorded the first certification); the counts themselves are the spine's
adoption record in each of the reference's family manifests.

The pin is Core `bd1a339f` (`0.43.1`, a **draft**, not released: a commit on Core's `main` inside its open `0.43.1`
slot, which no tag names), CHANGELOG.md and DECISIONS.md D13, D14 and D17. At that commit the crate answers every family with a manifest but two, which `tests/families.rs` names with
the reason: `encodingProfile` (declined, D8) and `identity` (not adopted, D14.7: its vocabulary is a test fixture of the
reference rather than conformance data, two of its lines need the `v1` rendering D8 declined, and the crate holds no IDL
vocabulary reader beyond the canonical-form slice). The capability and query families among those it answers:
`capabilityLaws`, all 144 vectors (validation, the invocation key, the declaration round-trip with its declared result,
the declaration refusals, enumeration, the pipeline type check), `capabilityDocumentLaws`, all 18 (the document value
space through validation, the key, the round trip and the type check), `capabilityGateLaws`, all 41 (the invocation's
identity, grant coverage, the named decision and dispatch, each decided under the file's context labels and again under
an integer context), `queryGrantLaws`, all 19 (the identity of a read and whether a grant covers it, over four
declarations read through the crate's declaration reader, each coverage also decided by the query registry), and
`queryDeclarationLaws`, all 84, answered at Core `bd1a339f` (the declaration reader's refusals on code and path; the
admission gate, registration, argument validation and the registry's gated dispatch on the verdict and the query-error
document, read through the seam codec and never compared as a sentence; D17). These five families run on the standard
profile and again on the embedded one, the library built without `std`; the embedded run executes on the build host,
and the `thumbv7em-none-eabihf` build is a build. `tests/query.rs` keeps what no vector pins: each refusal's sentence,
and the few codes and values no vector reaches (D17.9, D17.10).

On the embedded profile the decision path allocates nothing once the registry is built, for a capability and for a
read: `decide_named` and `dispatch` (allowed, needing approval, granted, parked or denied), the identity written into a
host's own buffer, and a grant's coverage are held at zero allocations by `tests/capability_alloc.rs`, a counting
allocator around each call (DECISIONS.md D12 and D14.8, which also say what still allocates: building a registry,
reading a tree argument, the owned argument pairs a caller presents, and a filtered read's identity asked of the
declaration alone rather than of the registry that rendered its filter). One function decides at both registries
(D14.6). A document argument is read to decide its membership and to spell it, so it allocates in proportion to the
document; the same leg measures and reports it (D13.8). The crate requires `alloc` on every profile. This statement
certifies against Core `bd1a339f` (`0.43.1`, draft, not released); the crate's version is `0.0.5-alpha.1`
(CHANGELOG.md).

These claims are checked at two Rust versions: exactly 1.85.0, the minimum `Cargo.toml` declares
(`rust-version = "1.85"`, a missing patch component read as `.0`), and stable, each by its own CI job. The minimum job
fails unless the compiler that ran its legs reports exactly 1.85.0, so a later patch never passes as the minimum
(DECISIONS.md D16). At both, the crate builds with every feature and without
`std`, builds for `thumbv7em-none-eabihf`, and passes every conformance leg, every premise and every row of the
allocation table, in debug and in release, hosted and without `std`. The format check and the lints run at stable only
(DECISIONS.md D15).

## Build

```powershell
pwsh ./run.ps1
```

`run.ps1` checks formatting, runs clippy, builds, runs the tests (the conformance legs among
them, and the capability and query legs and the allocation leg a second time without `std`, the allocation leg in
release too), and builds the `no_std` profile for `thumbv7em-none-eabihf` (Cortex-M4F). Then it runs the same build,
tests and `no_std` build at exactly the minimum Rust version `Cargo.toml` declares (1.85.0), on the installed toolchain
whose compiler reports that version. When none does, the run names 1.85.0 as missing with the commands that install
it, and stays green with the minimum reported as missing; if another patch of 1.85 is installed, the legs run there,
and every stage and the last line say it is not the minimum. `-MinimumOnly` runs only the minimum's legs and fails
unless 1.85.0 itself is installed; `-SkipMinimum` leaves them out.

## Verification

`verification/` holds the machine-checked proofs over the spine, and
[verification/README.md](verification/README.md) says what each proves, at what cost, and what it
does not: a Creusot proof that the canonical escape agrees with the reference's model of it, and
the Kani harnesses over the limited reader with the measured frontier they reach. They are opt-in,
because the checkers run on Linux:

```powershell
pwsh ./run.ps1 -Verify
```

runs each natively when its tool is on PATH, else in Docker from the pinned `Dockerfile` beside it,
and skips with a named reason when neither is available. A proof that runs and does not check fails
the run.

## Licence

Apache-2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
