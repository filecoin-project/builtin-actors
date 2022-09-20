TEST_CONTRACTS_DIR = tests/contracts
TEST_CONTRACTS_SOL = $(shell find $(TEST_CONTRACTS_DIR) -type f -name "*.sol")
TEST_CONTRACTS_HEX = $(TEST_CONTRACTS_SOL:.sol=.hex)

.PHONY: all
all: test-contracts

.PHONY: solc
solc:
	@if [ -z "$(shell which solc)" ]; then \
		echo "Please install solc, the Solidity compiler. See https://github.com/crytic/solc-select"; \
		exit 1; \
	fi

# Compile all Solidity test contracts.
# This could also be achieved with https://docs.rs/ethers/latest/ethers/solc/
.PHONY: test-contracts
test-contracts: $(TEST_CONTRACTS_HEX)

# Compile a Solidity test contract
$(TEST_CONTRACTS_DIR)/%.hex: $(TEST_CONTRACTS_DIR)/%.sol | solc
	@# Generate the .hex file which is the same as the .bin, but .bin would be renamed by solc to use CamelCase.
	@# We could use just the .bin files in the test, but this is a way to stick to the existing pattern.
	solc --bin $< | tail -n +4 | tr -d '\n' > $@
	solc --bin --abi --storage-layout --hashes --overwrite $< -o $(TEST_CONTRACTS_DIR)
