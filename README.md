# fuaran-core-rs

The Fuaran Core spine in Rust: canonical JSON with its proved grammar and parse limits,
the number form, SHA-256 content addresses, tree and op apply, the op-stream chain, the
DAG with its reconcile and diff, and the columnar, capability, propagation and seam
contracts. It answers every conformance family the .NET reference emits with a manifest (DECISIONS.md D8), and certifies against the conformance vectors the .NET reference
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
| `capability`, `seam` | the invocable-capability contract (argument validation, the invocation key, the declaration codec, the registry); the seams' refusal and declaration documents, read by one declaration-driven codec |
| `canonical_form`, `profile` | an IDL vocabulary's canonical form (aliases, presence, defaults); the wire profile `<name>@<major>.<minor>` |

## Conformance

`tests/` certifies the crate against the vectors the reference emits, one file per group of families, read from a
checkout of [fuaran-core](https://github.com/Fuaran-Core/fuaran-core) at the commit `core-pin.json`
names: clone it beside this repository as `fuaran-core/` (or set `FUARAN_CORE_DIR`). The legs fail
when no checkout is found. Which families are run, with each leg's count, is
[DECISIONS.md](DECISIONS.md) D8 (D6 recorded the first certification); the counts themselves are the spine's
adoption record in each of the reference's family manifests.

## Build

```powershell
pwsh ./run.ps1
```

`run.ps1` checks formatting, runs clippy, builds, runs the tests (the conformance legs among
them), and builds the `no_std` profile for `thumbv7em-none-eabihf` (Cortex-M4F).

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
