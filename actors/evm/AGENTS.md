# EVM actor (actors/evm/, with eam/, ethaccount/, placeholder/)

The EVM actor interprets Ethereum bytecode inside the FVM (FIP-0054). Each contract is its own EVM actor; the EAM (f010) creates them and assigns their f410 addresses. Contract developers expect Ethereum semantics, Ethereum tooling depends on them, and Lotus's Ethereum JSON-RPC encodes the same conventions, so a semantic change here is a protocol change and needs a FIP.

## Execution model

- The interpreter doesn't meter EVM gas. `GAS` returns the FVM gas left, the 63/64 rule applies to FVM gas, and a `CALL` with zero gas and non-zero value, or 2300 gas and zero value, gets 10M FVM gas so Solidity `transfer` works (`interpreter/instructions/call.rs`). Gas costs follow the WASM the interpreter executes, so storage access (KAMT reads and writes) dominates contract cost.
- `CALL` and `STATICCALL` become FVM sends of `InvokeContract` (STATICCALL read-only); `DELEGATECALL` runs the target's bytecode in this actor through `InvokeContractDelegate`. Native actors can call contracts through `handle_filecoin_method(uint64,uint64,bytes)`.
- Storage is a KAMT (bit width 5, bucket size 1) keyed by unhashed 32-byte big-endian slots, keys and values stored zero-stripped (`interpreter/system.rs`, `shared/src/uints.rs`). go-kamt-ipld and Lotus read it directly, so the layout must stay bit for bit identical. Before every outbound call `System` flushes state and records `saved_state_root`, then reloads if re-entry changed it (`interpreter/system.rs`); a new state mutation must clear `saved_state_root` or a re-entrant call loses writes. Transient storage (EIP-1153) lives in a second KAMT tagged with the message origin and nonce.
- `SELFDESTRUCT` sends the balance away and sets a tombstone immediately; the contract behaves normally until the top-level message ends, then reads as empty. It stays in the state tree and CREATE or CREATE2 through the EAM can resurrect it. These are pre-EIP-6780 semantics.
- Logs become FVM events with keys `t1` to `t4` and `d`, omitting `d` for empty data (`interpreter/instructions/log_event.rs`); Lotus parses exactly these keys for `eth_getLogs`.

## Ethereum compatibility

- Opcodes cover Shanghai plus EIP-1153, EIP-5656 and EIP-7939. `CALLCODE`, blob opcodes and EOF are absent; code starting with `0xEF` is rejected. Deployed code is capped at 24 KiB.
- Filecoin reinterprets several context values: `COINBASE` is zero, `BLOCKHASH` is the tipset CID digest, `PREVRANDAO` is drand randomness, `GASLIMIT` is a constant and `CHAINID` comes from the host.
- Addresses of the form `0xff` + 11 zero bytes + u64 are FVM ID addresses; others map to f410 addresses in the EAM namespace. CREATE and CREATE2 derive addresses as Ethereum does. Native actors report `0xfe` as their code (FIP-0054).
- A new EIP lands through a FIP at the next actors version. Check what Lotus RPC and traces must change with it.

## Precompiles

`interpreter/precompiles/mod.rs` holds two tables: Ethereum precompiles at `0x00..00NN` (plus the `0x0100` special case) and Filecoin precompiles at `0xfe00..00NN`, each indexed by the last byte minus one.

- An Ethereum or RIP precompile goes only at its canonical address, with Ethereum-identical input, output and failure behaviour.
- A Filecoin precompile goes only in the `0xfe` range, at the next index. Indices are never reused, including disabled ones such as `0xfe..04`.
- Every reserved address without an implementation (for example `0x0a`, KZG) returns success with empty output and still receives any value sent. Implementing one changes what existing contracts observe, so the FIP must say so.
- A new reserved address outside the two ranges must be added to both `is_reserved_precompile_address` here and `EthAddress::is_precompile` (`shared/src/address.rs`, used by the EAM's `can_assign_address`) in the same change.
- A precompile costs the WASM it executes unless it calls a syscall. Pure-Rust implementations are preferred; a C dependency must build for wasm32 under `build.rs`'s clang check.
- Test against the published vectors (`precompile-testdata/`, from geth and the EIPs), including malformed input.

## Tests

- `src/` unit tests and `tests/*.rs` run on `MockRuntime`. Solidity fixtures in `tests/contracts/` are checked-in `solc` output; commit a changed source with its output. Pragmas differ (some pin 0.8.25, some need 0.8.26 or later) and `make test-contracts` uses one `solc` whose failure leaves an empty `.hex` without failing, so build the changed fixture with a compiler its pragma accepts and check the output.
- `integration_tests/src/tests/evm_test.rs` runs on `test_vm` (see the root guide, Testing). Gas, call depth, memory and event behaviour need the Lotus FEVM itests (`itests/fevm_*`, `itests/eth_*`).
