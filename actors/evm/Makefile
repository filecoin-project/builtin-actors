TEST_CONTRACTS_DIR = tests/contracts
TEST_CONTRACTS_SOL = $(shell find $(TEST_CONTRACTS_DIR) -type f -name "*.sol")
TEST_CONTRACTS_HEX = $(TEST_CONTRACTS_SOL:.sol=.hex)

MEASUREMENTS_DIR  = tests/measurements
MEASUREMENTS_JSON = $(shell find $(MEASUREMENTS_DIR) -type f -name "*.jsonline")
MEASUREMENTS_PNG  = $(MEASUREMENTS_JSON:.jsonline=.png)

.PHONY: all
all: \
	test-contracts \
	measure-storage-footprint \
	plot-measurements

# Compile all Solidity test contracts.
# This could also be achieved with https://docs.rs/ethers/latest/ethers/solc/
.PHONY: test-contracts
test-contracts: $(TEST_CONTRACTS_HEX)

# Compile a Solidity test contract
$(TEST_CONTRACTS_DIR)/%.hex: $(TEST_CONTRACTS_DIR)/%.sol | solc
	solc --bin $< | sed '4q;d' | tr -d '\n' > $@

$(TEST_CONTRACTS_DIR)/callvariants.hex: $(TEST_CONTRACTS_DIR)/callvariants.eas $(TEST_CONTRACTS_DIR)/callvariants_body.eas
	eas $(TEST_CONTRACTS_DIR)/callvariants.eas | tr -d '\n' > $(TEST_CONTRACTS_DIR)/callvariants.hex

# Run storage footprint tests.
.PHONY: measure-storage-footprint
measure-storage-footprint:
	cargo test --test storage_footprint


# Render measurement charts.
.PHONY: plot-measurements
plot-measurements: $(MEASUREMENTS_PNG)

# Render a specfic plot if the data changed.
$(MEASUREMENTS_DIR)/%.png: \
	$(MEASUREMENTS_DIR)/%.jsonline \
	$(MEASUREMENTS_DIR)/storage-footprint.plt \
	$(MEASUREMENTS_DIR)/storage-footprint.sh \
	| jq gnuplot
	cd $(MEASUREMENTS_DIR) && ./storage-footprint.sh $*


# Requirements checks.

.PHONY: solc
solc:
	@if [ -z "$(shell which solc)" ]; then \
		echo "Please install solc, the Solidity compiler. See https://github.com/crytic/solc-select"; \
		exit 1; \
	fi

.PHONY: gnuplot
gnuplot:
	@if [ -z "$(shell which gnuplot)" ]; then \
		echo "Please install gnuplot. See http://www.gnuplot.info/"; \
		exit 1; \
	fi

.PHONY: jq
jq:
	@if [ -z "$(shell which jq)" ]; then \
		echo "Please install jq. See https://stedolan.github.io/jq/"; \
		exit 1; \
	fi
