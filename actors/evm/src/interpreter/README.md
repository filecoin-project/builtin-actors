## Introduction

This is the EVM interpreter used in the Filecoin network.

## History

This interpreter was incubated under [fvm-evm](https://github.com/filecoin-project/fvm-evm/).
It was initially based on [evmodin](https://github.com/vorot93/evmodin) (whose [LICENSE](./LICENSE) has been transferred here),
but has diverged significantly.

## Divergences

This is a non-comprehensive list.

- Because this interpreter does not service an Ethereum network, we were able to remove historical baggage and then
  tracking of which opcodes and precompiles were introduced at which forks. This interpreter supports the Berlin hardfork.
- Removed support for continuations. We don't expect to use this feature in FVM.
- Removed support for tracing. We may want to re-introduce this at some point proxying over to the debug::log syscall.
- All instructions under instructions/ have been `#[inlined]`.
- The Host trait has been removed and substituted by a System concrete type that uses the FVM SDK (and thus depends
  on the actor Wasm sandbox). We will likely need to restore this trait for unit testing purposes.
- The Memory is now backed by BytesVec instead of a Vec; it exposes a method to grow it, but we need to check that it's
  being called from every possible point.
- `Message#code_address` has been removed; we may need to reintroduce when we start handling delegate call.
  Bytecode processing has lost features, e.g. code padding (was it an implementation detail?), only-once jumpdest table 
  derivation and persistence, etc. This is connected with the removal of continuations and other features.
- Many code layout/structure changes and refactors.