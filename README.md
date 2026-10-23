# fuaran-core-rs

The Fuaran Core spine in Rust: canonical JSON with its proved grammar and parse limits,
the number form, SHA-256 content addresses, tree and op apply, the op-stream chain and
the DAG. It certifies against the conformance vectors the .NET reference
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

## Conformance

`tests/conformance.rs` certifies the crate against the vectors the reference emits, read from a
checkout of [fuaran-core](https://github.com/Fuaran-Core/fuaran-core) at the commit `core-pin.json`
names: clone it beside this repository as `fuaran-core/` (or set `FUARAN_CORE_DIR`). The legs fail
when no checkout is found. Which vectors are run, and which are not the spine's, is
[DECISIONS.md](DECISIONS.md) D6.

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
