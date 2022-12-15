# Tool for applying test vectors from Ethereum on FEVM

* [devgrants/fvm-ethereum-test-vectors.md at fvm_ethereum_test · storswiftlabs/devgrants · GitHub](https://github.com/storswiftlabs/devgrants/blob/fvm_ethereum_test/rfps/fvm-ethereum-test-vectors.md)

* [Proposal](https://docs.google.com/presentation/d/1u_-CamlnGZAVuY2ci3JSNnFq51l4X_TH/edit?usp=sharing&ouid=105194677015683983388&rtpof=true&sd=true)


## Howto Run the test

1, Pull the Eth Test vectors (`https://github.com/ethereum/tests.git`)

```
git submodule update --init
```

2, Launch the test for single test vector json file.

```
RUST_LOG=trace \
	VECTOR=test_evm_eth_compliance/test-vectors/tests/GeneralStateTests/stCallCodes/callcall_00.json \
	cargo run -p test_fevm_eth_compliance \
	-- statetest
```

