# EVM

The EVM actor is a Wasm implementation of the EVM bytecode interpreter, originally prototyped in https://github.com/filecoin-project/fvm-evm/

## Testing

The `tests` library contains integration tests, some of which use Solidity contracts. The compiled versions of these contracts are checked into [tests/contracts](./tests/contracts). To modify them, you will need to install the [solc](https://docs.soliditylang.org/en/latest/installing-solidity.html) and optionally [solc-select](https://github.com/crytic/solc-select), then from this directory run the following command to generate the new artifacts:

```shell
make test-contracts
```
