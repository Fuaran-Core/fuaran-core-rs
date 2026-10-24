# Verification

Machine-checked proofs over the spine, and the record of what each cost and where each
stopped. Both checkers run on Linux; `pwsh ./run.ps1 -Verify` runs each natively when its
tool is on PATH, else in Docker from the pinned `Dockerfile` beside it, and skips with a
named reason when neither is available. A proof that runs and does not check fails the run.

| Leg | What is proved | Checker | Cost to check |
|---|---|---|---|
| `creusot/escape` | the crate's own `json::escape::escape_bytes`, the loop `escape_string` runs, returns exactly the reference proof's `quoted` of its input | Creusot 0.9.0 · Why3 1.8.2 · cvc5 1.3.1, cvc4 1.8, Z3 4.8.12, Alt-Ergo 2.6.2 | replay 1 s; a fresh search was 1 min 22 s on 16 cores (Phase 486, not re-measured) |
| `kani` | `escape_string` on the empty string and on every one-byte ASCII string returns, quoted, exactly as long as rule 6 says (its UTF-8 check stubbed, below) | Kani 0.68.0 · CBMC 6.11.0 | 0.5 s + 2.1 s |
| `kani` (feature `frontier`) | the escape on two and three symbolic bytes; the limited reader: panic-freedom, every bound honoured, a stated allocation ceiling | Kani 0.68.0 · CBMC 6.11.0 | **does not close** on 16 GB; the measurements are below |

This directory is the outcome of Phase 486, an exploration whose question was whether the
Rust spine's parser and limits can be proved directly, and at what cost. The answer is split:
the writer's pure functions can, cheaply, under Creusot; the reader cannot, under Kani as it
stands, and the reason is measured rather than guessed.

## Creusot: the canonical escape agrees with `WireCanon.fst`

**Claim.** `escape_bytes(s) == quoted(s)` for every byte string `s`, where `quoted`, `escape`
and `esc_ch` are Pearlite logic functions written clause for clause from the reference proof's
`quoted` / `escape` / `esc_ch` (`proofs/WireCanon.fst` in the Core repository), over UTF-8
bytes in place of the F\* model's abstract alphabet. `escape_bytes` is the crate's own function
(`src/json/escape.rs`), and `json::escape_string` is that function over the UTF-8 encoding of its
argument. Rule 6 escapes only ASCII — `"`, `\` and U+0000–U+001F, each one UTF-8 byte that never
continues a sequence — so the escape of a string's bytes is the UTF-8 encoding of the escape of
its characters, and the byte-level claim is the character-level one.

**How the proof reaches the crate's function.** The proof crate (`creusot/escape/`) compiles
`src/json/escape.rs` as one of its own modules (`#[path]`), so the lines it checks are the lines
the spine builds. The specification sits on those lines under `cfg(creusot)` — an `ensures` on
`escape_bytes`, a `requires`/`ensures` on `hex_digit`, the loop's two invariants and its
`proof_assert!` steps — which only the Creusot build sets: every other build of the crate sees
none of it and takes no dependency. The logic functions and the lemmas stay in the proof crate.
The file holds `escape_bytes` and nothing else the proof would have to consume, because Creusot
cannot compile the rest of `src/json.rs` (the reader's generic `Parser<B: Build>`; see the
verdict below).

**What was checked.** 42 goals, all closed: `escape_bytes` (29: the loop invariant's entry and
preservation, every bounds and overflow check, the capacity computation, the postcondition),
`escape_push_back` (10: the induction that escaping one more byte appends that byte's escape —
the same induction `read_str_inverts_escape` runs in the F\* proof), `concat_laws`, `escape` and
`hex_digit`. The record of which prover closed which goal is `creusot/escape/verif/**/proof.json`,
which `cargo creusot prove --replay` replays in about a second; a fresh search (`cargo creusot
prove`) rewrites it. Changing the crate's function breaks the replay: a planted upper-case hex
digit in `hex_digit` fails `spine_escape.hex_digit` with `1 unproved file`.

**What it does not prove.** Two gaps, both stated here so nobody reads more into the claim:

1. **Agreement with the F\* model is by transliteration.** The Pearlite `esc_ch` / `escape` /
   `quoted` mirror the F\* definitions clause for clause, but no tool checks that mirror; the
   alphabets differ (`u8` here, the abstract `ch` there), so a mechanical bridge would be a
   proof of its own.
2. **That `escape_string` returns well-formed text is not proved.** The escape replaces only
   ASCII bytes, each with ASCII ones, and copies every other byte in order, so UTF-8 in is UTF-8
   out; `escape_string` converts through the checked `String::from_utf8` rather than an `unsafe`
   conversion (the crate holds no `unsafe`), and that check's failing arm is unreachable by the
   argument, not by a proof.

**The replay identity.** The reference's claims ladder records this proof as a row of its own,
and a host-compliance declaration cites that row with the toolchain and the replay identity the
row records (Core's `docs/host-compliance.md`, class `proof-replayed`). The identity is
`sha256:` and the SHA-256 of a manifest with one line per file of the record — the proved
function `src/json/escape.rs`, its specification `creusot/escape/src/lib.rs`, and every
`creusot/escape/verif/**/proof.json` — in ordinal order of the repository-relative path, each
line `<SHA-256 of the file, lower-case hex>  <path>` and a newline: what GNU `sha256sum` prints.
`creusot/replay-identity.ps1` computes it, and `run.ps1 -Verify` prints it. Any change to one of
those files moves it, so a row recorded against one tree does not answer for another. The
toolchain identity is `creusot@0.9.0 why3@1.8.2 cvc5@1.3.1 cvc4@1.8 z3@4.8.12 alt-ergo@2.6.2`,
the versions `creusot/Dockerfile` pins and the image runs. (The record labels some goals
`z3@4.15.3`, the version in why3find's calibration profile, `why3find.json`; the binary the image
runs and the replay uses is Debian's 4.8.12.)

**Cost.** Toolchain: about 25 minutes and three attempts in a fresh `rust:1-bookworm`
container — the opam switch refused to build `why3-ide`'s GTK dependencies until they were
installed first, the installer's own Z3 (4.15.3) wants glibc 2.38 where bookworm has 2.36, so
Debian's Z3 4.8.12 is passed with `--external z3`, and Why3 1.8.2 does not recognise cvc5
1.3.1's version string but drives it anyway. `creusot/Dockerfile` is that route, pinned.
Proof engineering: about 25 minutes and eight prove cycles from a first translation to a closed
proof, for a 40-line program function with eight `proof_assert!`s and two lemma calls. Four
things cost cycles and are worth knowing:

- A `#[logic]` body needs `pearlite! { … }` before `@` or closures parse.
- **Creusot 0.9.0 gives a `match` arm's default branch none of the earlier arms' negations**
  (the Coma reads `default -> (! bb16)` with no condition), so the last arm of a `match` on
  the byte could not be shown to be the verbatim case. The dispatch is an `if` chain, which
  carries both sides of each test.
- The solver needed the sequence laws stated as hypotheses — `a ++ (c ++ [x]) == (a ++ c) ++ [x]`,
  `a ++ [x] == a.push_back(x)`, `a ++ [] == a` (`concat_laws`) — although Why3's `seq` theory
  carries them as rewrite lemmas, and `escape` written as explicit recursion with its own
  `push_back` lemma closed where the library's `flat_map_push_back` route left the step open.
- `cargo creusot prove` names a failing goal only as `[assertion]`. `--why3session` writes a
  session whose leaf order follows the program, and the `.coma` file's `[%#slibN]` labels
  carry the source spans, which together locate it.

**Verdict: adopt, for the writer's pure functions.** Cost of the order of an hour per function
of this size, a two-second replay as the gate, a pinned Docker toolchain for any machine. The
next candidates are the canonical number layout and the member-order comparison. Not for the
reader: its generic `Parser<B: Build>` over `Vec` and `String` would need the crate compiled
under Creusot, which is a port, not a harness.

## Kani: the limited reader, and where it stops

**Claim attempted.** For every valid UTF-8 text of at most N bytes and every limits record
(the depth concrete, every other bound symbolic), the reader returns; an accepted value is
within every bound it was read under; a breach names the bound it passed with that bound's
declared value; a document over `documentBytes` is refused before a byte of heap is taken; and
the peak live heap, counted by a global allocator in the harness, stays under a stated ceiling.
The harnesses are `kani/src/harnesses.rs` behind the `frontier` feature, running the real reader
(`scan`) at a heap-light value model (`json::kani_support::Shape`, under `cfg(kani)`) so that
`JVal`'s recursive drop glue is out of the picture, with three stubs: `f64::from_str`
over-approximated by any `f64`, `format_number` asserted unreachable below sixteen bytes, and
`format!` modelled as the empty message.

**Measured.** Every rung was run alone in a 16-CPU, 15 GB Linux container (`kani/Dockerfile`'s
toolchain), CBMC with `--unwind` N+2:

| Rung | Input | Outcome |
|---|---|---|
| `limits_n4_d0` | 4 symbolic bytes, symbolic bounds | 14.7 GB resident at 4 min 32 s, killed |
| `limits_n4_d1` | as above | 10.5 GB at 16 min, unfinished, killed |
| `limits_n2_d0` | 2 symbolic bytes, symbolic bounds | out of memory at 3 min 28 s |
| `limits_n2_d0`, ASCII-only bytes, no UTF-8 validation | 2 symbolic bytes | 14.3 GB at 2 min 12 s, killed |
| `limits_n2_d0`, every bound concrete | 2 symbolic bytes, depth only | out of memory at 2 min 1 s |
| `limits_n1_d0` | **1** symbolic byte, symbolic bounds | out of memory at 2 min 17 s |
| `limits_n1_d0`, **no counting allocator**, no heap assertions | 1 symbolic byte | out of memory at 3 min 11 s |
| `read_n1_d0` (the public `read`, real value model) | 1 symbolic byte | 7 minute timeout, unfinished |
| control: `read("7")`, one concrete byte | concrete | **verified**, 5.7 s |
| control: `read("[]")`, two concrete bytes | concrete | 7 minute timeout, unfinished |
| control: `escape_string` on every 1-byte ASCII string | 1 symbolic byte | **verified**, 6.6 s |
| `escape_len0` / `escape_len1` | the empty string; every one-byte ASCII string | **verified**, 13.4 s and 6.0 s |
| `escape_len2` / `escape_len3` | every 2- and 3-byte ASCII string | out of memory at 1 min 18 s and 1 min 37 s |
| `escape_n1`, symbolic length 0..=1 | 1 symbolic byte, symbolic length | out of memory |

**Re-run over the heap-stack reader (Phase 567, `DECISIONS.md` D10).** The reader no longer recurses and keeps its
nesting in one allocation that never grows. Each rung was run alone over both readers in the same image, under a
14 GiB cap, so the two columns compare:

| Rung | Recursive reader | Heap-stack reader |
|---|---|---|
| `limits_n1_d0` | out of memory at 1 min 21 s; 760,995 program steps | out of memory at 1 min 32 s; 958,425 steps |
| `limits_n2_d0` | out of memory at 1 min 53 s; 1,129,282 steps | out of memory at 1 min 41 s; 1,356,099 steps |
| `read_n1_d0` | out of memory at 5 min 12 s; 3,961,189 steps | out of memory at 5 min 20 s; 4,165,522 steps |
| `limits_n2_d1` | — | 9 minute timeout, unfinished |
| `read` of a concrete text under `STRICT`, with and without the counting allocator | `7`: 5 minute timeout, unfinished | `7`, `[]`, `[[0]]`: 5 minute timeout, unfinished |

No rung closes. The depth-0 rungs never open a container, so the nesting stack was never their cost; the strings and
items a read grows are, as above. (The concrete controls here time out over both readers where the earlier `read("7")`
control verified, so they differ from it in a way not chased down; they discriminate nothing between the readers.)

So the checker is fine on this crate (the controls), the UTF-8 validation is not the cost
(ASCII-only fails the same way), the symbolic bounds are not the cost (concrete bounds fail the
same way), and the counting allocator is not the cost (it fails without one). What fails is
anything that grows a `String` or a `Vec` under symbolic content: a concrete `[]` does not
finish in seven minutes where a concrete `7` takes six seconds; one symbolic byte into the
reader — which can open an array or an object — exhausts 15 GB; and even the writer's escape,
straight-line code, closes on one symbolic byte and runs out of memory on two, or on one byte
of symbolic length (a symbolic length makes the output's capacity symbolic). The cost sits in
the growth path through `alloc::raw_vec` (every `assume(false)` abort path in CBMC's log is in
`raw_vec::handle_error` and the allocation error handler) multiplied by the symbolic position
every index then carries.

**What closes.** `escape_len0` and `escape_len1`: `escape_string` on the empty string and on
every one-byte ASCII string returns, is quoted, and is exactly as long as rule 6 says — 0.5 s and
2.1 s of verification time. Thin, and stated as such; it is what a 16 GB machine proves of this
crate with Kani. Since `escape_string` converts the bytes `escape_bytes` returns through the
checked `String::from_utf8`, the two rungs stub that conversion with the unchecked one
(`from_utf8_valid`): the validation's loop over one symbolic byte, measured, exhausts the memory on
its own (the rung's unwinding assertion fails at the default bound, and at a bound of 10 CBMC runs
out of memory), and the well-formedness it checks is the escape's argument, stated under the
Creusot leg's gaps, not these rungs' claim.

**Verdict: do not adopt for the reader; keep what closes.** The reader rungs and the longer
escape rungs stay in the tree behind the `frontier` feature, with these tables, so the next attempt — a bigger machine, a
newer Kani, or a reader whose strings and items do not grow under the checker (one arena for the nesting alone was
tried, and is the second table) — is one flag away and starts from the measurement rather than from a blank page. The
two closing rungs run under `-Verify`. The allocation ceiling in the harness (`PEAK_CEILING`) is a
bound to be found by lowering it until a rung fails; no rung has closed, so it is not a proved
figure and the write-up does not call it one.

## Running the legs by hand

```powershell
# Kani, natively (cargo kani on PATH) or in the pinned image
cd verification/kani; cargo kani                      # the two closing rungs
cd verification/kani; cargo kani --features frontier --harness limits_n1_d0   # one frontier rung
docker build -t fuaran-core-verify-kani verification/kani
docker run --rm -v ${PWD}:/src fuaran-core-verify-kani bash -c "cd /src/verification/kani && cargo kani"

# Creusot
docker build -t fuaran-core-verify-creusot verification/creusot
docker run --rm -v ${PWD}:/src fuaran-core-verify-creusot bash -c "cd /src/verification/creusot/escape && cargo creusot && cargo creusot prove --replay"
```

`cargo creusot prove` without `--replay` searches afresh and rewrites `verif/**/proof.json`;
commit the result when the proof changes.

## Where the escape proof is recorded

The reference's claims ladder (Core's `proofs.json`) carries the escape proof as the row
`spine-escape-quoted`, which names this repository, the proved function, the toolchain identity,
the replay identity and the commit it was recorded at, with the transliteration gap on its
boundary. A host-compliance declaration for this repository cites it as `proof-replayed`
evidence on that row, and the reference's validator checks the citation: the row is this host's
proof, and the toolchain and replay identity the declaration names are the ones the row records.
It does not run the prover; this leg does. A change to the proof's files moves the replay identity,
and the row is re-recorded in the reference in the change that lands the new proof.

`.github/workflows/verify.yml` runs `run.ps1 -Verify` on demand (`workflow_dispatch`, never on a
push), building the two pinned images on the runner.
