# builtin-actors agent guide

builtin-actors implements Filecoin's built-in actors: the miner, market, power, reward and other system actors, plus the EVM. They compile to a WASM bundle that every node runs inside the FVM, so their results, state layout and gas are consensus.

## Where builtin-actors sits in Filecoin

- [ref-fvm](https://github.com/filecoin-project/ref-fvm) executes the bundle. Actors reach it only through `fvm_sdk` syscalls, wrapped by `runtime/` (`FvmRuntime`). The FVM charges gas by instrumenting the WASM, so every change to actor code, dependencies, toolchain or the `wasm` profile changes gas.
- [go-state-types](https://github.com/filecoin-project/go-state-types) mirrors every state, params and return type in Go (`builtin/vN/`) and holds the state migrations (`builtin/vN/migration/`) and the Go invariant checks.
- [Lotus](https://github.com/filecoin-project/lotus) embeds the release bundles (`build/actors/`, `build/builtin_actors_gen.go`), wires migrations into network upgrades, and serves the Ethereum JSON-RPC that must agree with the EVM actor.
- [Forest](https://github.com/ChainSafe/forest) runs the same bundles and reads state through ChainSafe's [fil-actor-states](https://github.com/ChainSafe/fil-actor-states) mirror.

A change to state, params, a method or an event needs matching changes in go-state-types and Lotus, and usually Forest. Name them in the PR description. If a sibling checkout is available, read its `AGENTS.md` before changing a shared contract.

### Versions and network upgrades

The crate major is the actors version, normally one per network upgrade (`README.md`, Versioning). Actor code never branches on network version: each upgrade activates a new bundle, and a behaviour change lands in the next major behind an accepted FIP. Existing majors change only for a fix that must ship before the next upgrade.

A release is the version-bump PR: merging it builds `builtin-actors-<network>.car` for each network and attaches them to the GitHub release (`RELEASE.md`). Lotus picks a release up separately with `build/actors/pack.sh` and `make actors-gen`.

## Orientation

- `runtime/`: the `Runtime` trait, dispatch, policy, HAMT/AMT wrappers and `MockRuntime` (`test_utils`). `vm_api/`, `test_vm/`, `integration_tests/`: the scenario test stack. `state/`: cross-actor invariant checks. The root crate (`build.rs`) builds the bundle.
- Network parameters (sector sizes, proof types, durations) are compile-time: `BUILD_FIL_NETWORK` or a root feature selects mainnet, calibrationnet, butterflynet, devnet, testing and others, and `runtime/build.rs` turns that into `feature="..."` cfgs that aren't Cargo features; add a network parameter there. A plain build is mainnet.
- Any workspace build runs `build.rs`, which compiles every actor to wasm32 and needs a clang that targets wasm32 (for `blst` in the EVM actor).

### Before changing or reviewing these, read

- The EVM actor, EAM, ethaccount or placeholder: `actors/evm/AGENTS.md`
- Building and releasing: `CONTRIBUTING.md`, `RELEASE.md`

## Actors

- Singletons live at fixed ID addresses (`runtime/src/builtin/singletons.rs`). Code addresses them by constant. Miners, multisigs and payment channels are created through init `Exec`, EVM contracts through the EAM, and accounts and placeholders by the FVM on first send to a new f1, f3 or f410 address. A placeholder becomes an ethaccount when it first sends a message.
- Actor type numbers follow the order of `ACTORS` in `build.rs`, which must match `vm_api::builtin::Type` and the FVM's numbering. Adding, removing or reordering an actor renumbers types.
- Actors don't depend on each other's crates. A caller mirrors the callee's method numbers and params in its own `ext.rs`, so changing a method means updating every `ext.rs` that calls it.

## Methods and callers

- Every method calls exactly one `validate_immediate_caller_*` method; the runtime aborts a call that makes none or two.
- `rt.transaction` rejects sends and actor creation or deletion. Compute the state change inside it, return the work to do, and send after it. A send returns `Ok` even when the callee fails; wrap it in `extract_send_result` when that failure should propagate.
- Methods numbered from `FIRST_EXPORTED_METHOD_NUMBER` (2^24) are exported, numbered by the FRC-0042 hash of their name (`method_hash!`), and callable by contracts. Lower numbers reject EVM and non-builtin callers (except on the EVM and EAM, which dispatch unrestricted), but accounts, ethaccounts and multisigs can still call them, so the caller check is the real access control.
- Method numbers are permanent. A removed method's number is never reused, and renaming an exported method changes its number. Wallets, explorers, Lotus and Solidity callers decode params, returns, exit codes and events, so treat them as public API.
- Params of a method that users or contracts call keep their shape. A new shape, even one optional trailing field, gets a new method with a new number (miner's `PreCommitSectorBatch2`, `ExtendSectorExpiration2`, `ProveCommitSectors3`); don't make one method accept both shapes. A method whose caller check admits only built-in actors may change shape, with every `ext.rs` caller and the go-state-types mirror updated in the same bundle.
- Gas follows the WASM a method executes, and choices that look minor in Rust can amplify there: allocation, big-number arithmetic and generic code cost more than they appear (e.g. in the reward stream maths, `i128` costs about 5% of the equivalent `BigInt`). Weigh this on hot and per-epoch paths, and compare the built wasm when unsure; elsewhere, prefer the clearer code.
- A panic or arithmetic overflow aborts the call with `USR_ASSERTION_FAILED` (`panic = "unwind"` and `overflow-checks` in the `wasm` profile). An unchecked `unwrap` or `+` on user input is a caller-triggerable abort.
- Exit codes classify the fault. Caller-caused failures get the specific code (`USR_ILLEGAL_ARGUMENT`, `USR_FORBIDDEN`, `USR_NOT_FOUND`, `USR_INSUFFICIENT_FUNDS`); `USR_ILLEGAL_STATE` means broken state or a programmer error. For a condition other methods already check, return the code they return.
- Cron runs every epoch on every node and nobody pays for it. Keep its work bounded per epoch; work proportional to state belongs in user-paid methods or bounded batches. Cron runs once after all explicit messages in the tipset, so logic at an epoch boundary (`==` start or end) must hold for later messages in the same epoch; give each boundary comparison a test that fails if `<=` becomes `<`.
- An abort in an implicit message (cron, `AwardBlockReward`) can stall the chain, so each path's failure handling is specified: the reward ledger falls back to paying gas only, a gas reward the balance can't cover aborts, and power acts on a failed miner cron callback. Trace who consumes a failure and keep its specified handling; don't invent a fallback. Validate state when a user method writes it, so bad state fails that method rather than the next implicit call.

## Encoding and state layout

State, params and returns encode as DAG-CBOR and must match go-state-types byte for byte. The Rust compiler won't catch a wrong encoding; only paired golden tests do.

- Structs derive `Serialize_tuple`/`Deserialize_tuple` (`fvm_ipld_encoding::tuple`), never plain serde derives, which produce CBOR maps. Field order is wire order. Enums use `Serialize_repr` with an explicit `repr`.
- `BigInt` and its aliases (`StoragePower`, `DealWeight`, `Spacetime`) need `#[serde(with = "bigint_ser")]` to get Go's big-int bytes form. Without it they still compile and encode in num-bigint's array form. `TokenAmount` encodes correctly on its own.
- DAG-CBOR integers stop at 64 bits, and Go decodes them as `int64` or `uint64`, so a field's range must fit its Go type. An `i128` or `u128` field encodes only while the value fits and errors beyond that, which small test values never reach. Store anything that can outgrow 64 bits as `BigInt` or `TokenAmount`, and convert to a primitive for arithmetic.
- Consensus arithmetic is integer-only; Go and Rust must agree bit for bit, and floats appear only in tests. Fractions are fixed-point integers with an explicit denominator: Q.128 (`PRECISION` in `runtime/src/builtin/reward/math.rs`) for reward and smoothing maths, `DENOM` (10^18) for reward stream weights and shares. Reuse the denominator the surrounding code uses.
- DAG-CBOR allows only tag 42 (CIDs), which is why big integers use Go's bytes form rather than CBOR bignum tags. Strings must be valid UTF-8; market's `Label` is a string-or-bytes union so non-UTF-8 labels stay representable.
- Byte fields need `#[serde(with = "strict_bytes")]`, `RawBytes` or `BytesSer`; a bare `Vec<u8>` encodes as an array of integers.
- `#[serde(transparent)]` on a one-field struct encodes the bare value, which matches a Go alias; without it, a one-element array, which matches a Go struct. Follow go-state-types.
- A state shape change ships with a migration and the go-state-types mirror in the same upgrade, so every entry has one shape. A lazy migration (an optional trailing field, `#[serde(default)]` in Rust and `cborgen:"optional"` in Go, written as entries are rewritten) leaves two shapes on chain for good and complicates every reader. It is a last resort for state too large to migrate, and the FIP must call it out, as FIP-0100 did for `SectorOnChainInfo` and `ExpirationSet`.
- HAMT blocks don't record their bitwidth; AMT roots do, but readers still expect the structure's. Reuse the structure's named bitwidth or config constant, since bitwidths differ per structure. Keep the key encoding too: `MapKey` for `u64` is unsigned LEB128 and for `i64` is zigzag, and Go uses `abi.UIntKey` and `abi.IntKey` respectively. Switching a key type or helper strands every existing entry.
- Golden hex vectors in `actors/{miner,power,reward}/tests/types_test.rs` have Go twins in go-state-types `builtin/vN/*/*_types_test.go`. A new or changed type gets a vector in both, covering zero big-ints, nulls, empty lists and, for a lazily migrated struct, both lengths.

## Testing

- Actor unit tests (`actors/<name>/tests/`) use `MockRuntime`, which fails on any send, gas charge, event or caller validation the test didn't expect.
- Scenario tests are `#[vm_test]` functions in `integration_tests/src/tests/`. They run only through a wrapper in `test_vm/tests/suite/` listed in its `mod.rs`; add both. A test without a wrapper compiles and never runs.
- `test_vm` runs actors as native Rust: gas is free, events and memory are unchecked and proof verification is mocked (a proof equal to `TEST_VM_INVALID_POST` fails). A green run doesn't exercise gas, event size, WASM limits or call depth. Check those in a real FVM: a Lotus itest with `LOTUS_BUILTIN_ACTORS_V<N>_BUNDLE=<path to a built .car>`.
- A test whose expected consensus value moves (minting, fees, power, pledge) records a behaviour change. Name it in the PR; don't quietly update the constant.
- Invariants live in each actor's `testing.rs` and in `state/src/check.rs`, and run only when a test calls them (`assert_invariants`). Real chain state is checked with the Go invariants in go-state-types (`lotus-shed check-invariants`, `migrate-state --check-invariants`). New state needs invariants in both.

### Commands

```sh
make rustfmt   # cargo fmt --all --check
make check     # clippy with and without --all-targets, -D warnings
make test      # cargo nextest run --workspace
cargo test -p fil_actor_miner
cargo test -p test_vm --test all_tests <name>
make bundle-devnet   # output/builtin-actors-devnet.car
```

CI also runs cargo-deny, cargo-shear and a reproducible bundle build for every network (`.github/workflows/`).

## Change hygiene

Add tests that would fail for a plausible regression. Report suspected vulnerabilities privately through the repository's GitHub security policy, never in a public issue or PR.

### Comments, docs and commit messages

- Write code comments and reference docs year-zero: describe the code as it is, as if it had always been this way. Release notes may compare old and new behaviour. History (old behaviour, the bug, the fix, the issue) goes in the commit message and PR description. In comments, words like "now", "previously", "used to", "no longer", "instead of" and "fixed" signal change narration; rewrite or delete. A link that explains a constraint that still applies stays.
- Actor code is one actors version, exactly what that bundle executes. Lotus, ref-fvm and Forest carry version guards because they run all of history; this code carries one version. Leave change records ("removed by FIP-0XXX", "added in v17", "was X") out of code and tests. A comment may explain an oddity the code must keep, such as a quirk preserved for existing state, or flag a planned removal.
- A comment, in code or tests, earns its place by stating what a maintainer can't read from the code: intent, an invariant, a compatibility or consensus constraint, a surprising cost. Don't restate the code, the test name or the assertion; prefer one precise sentence. Public API docs state the contract: inputs, outputs, errors, bounds.
- Commit messages and PR descriptions explain what changed and why, for a reviewer. Skip summaries of the diff, boilerplate sections and claims you haven't verified.

### Pull requests

PR titles and when FIP work merges follow `CONTRIBUTING.md`; the merge queue squashes to the title.

## Reviewing changes

Read the relevant callers, `ext.rs` mirrors and tests before commenting, and verify assumptions against current code.

Priorities, in order: encoding and state layout; caller validation and access control; consensus behaviour outside an accepted FIP; gas and bounded work; Go and downstream parity; tests and invariants. The sections above define each.

Comment only on a concrete, plausible failure the change introduces: the triggering input or state, the wrong result, its consequence. Skip style preferences, speculative hardening, pre-existing problems and refactors without behavioural benefit. One exception: this repository treats comments that narrate the change or restate the code as defects, so report them (see Comments, docs and commit messages).

## Maintaining agent context

This file and the `AGENTS.md` files it lists are maintained with the code. When a change makes one of them wrong, fix it in the same PR. When a change alters a contract with a neighbouring repository, name the follow-up that repository needs, including its `AGENTS.md`, in the PR description.

Each `AGENTS.md` has a sibling `CLAUDE.md` containing only `@AGENTS.md`, because by default Claude Code skips `AGENTS.md` files whenever a `CLAUDE.md` exists at or above its working directory. Add the pair together; put content in `AGENTS.md`, never in the `CLAUDE.md`.
