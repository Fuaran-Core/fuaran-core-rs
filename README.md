# fuaran-core-rs

The Fuaran Core spine in Rust: canonical JSON with its proved grammar and parse limits,
the number form, SHA-256 content addresses, tree and op apply, the op-stream chain and
the DAG. It certifies against the conformance vectors the .NET reference
([fuaran-core](https://github.com/Fuaran-Core/fuaran-core)) emits.

The crate is `no_std` + `alloc` by default, so it builds for microcontroller targets;
`std` is a feature.

## Status

Under construction (Phase 463): the spine is being moved here from the Rust UI host.

## Build

```powershell
pwsh ./run.ps1
```

`run.ps1` checks formatting, builds, runs the tests, and builds the `no_std` profile
for `thumbv7em-none-eabihf` (Cortex-M4F) when that target is installed.

## Licence

Apache-2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
