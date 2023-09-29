# EVM Precompile Test Data

These data files come from [go-ethereum](https://github.com/ethereum/go-ethereum/tree/master/core/vm/testdata/precompiles) and are therefore licensed under the LGPLv3. However, they're not included in published crates (they're excluded by cargo) or build artifacts. They're only loaded at runtime by a few unit tests, and therefore trivially meet the requirements of the LGPL.
