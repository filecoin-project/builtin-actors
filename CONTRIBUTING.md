# Contributing to Filecoin Built-in Actors

Welcome to the Filecoin built-in actors repository! This codebase serves as the canonical implementation of on-chain actors that power the Filecoin network and is a common good across all Filecoin client implementations.

## Getting Started

### Prerequisites

- **Rust**: This project uses Rust 2024 edition. Install via [rustup](https://rustup.rs/)
- **Make**: Required for build automation
- **Docker**: Required for reproducible builds
- **Git**: For version control

### Repository Structure

```
builtin-actors/
├── actors/           # Individual actor implementations
│   ├── account/      # Account actor
│   ├── cron/         # Cron actor
│   ├── miner/        # Storage miner actor
│   ├── market/       # Storage market actor
│   ├── evm/          # Ethereum Virtual Machine actor
│   └── ...           # Other actors
├── runtime/          # Shared actor runtime utilities
├── test_vm/          # Test virtual machine
├── integration_tests/ # Cross-actor integration tests
└── state/            # State management utilities
```

## Development Setup

1. **Clone the repository:**
   ```bash
   git clone https://github.com/filecoin-project/builtin-actors.git
   cd builtin-actors
   ```

2. **Install the toolchain:**
   ```bash
   make toolchain
   ```

3. **Install testing dependencies:**
   ```bash
   make install-nextest
   ```

4. **Verify your setup:**
   ```bash
   make check
   make test
   ```

## Making Contributions

### Types of Contributions

1. **FIP Implementation**: Implementing Filecoin Improvement Proposals - many contributions involve implementing protocol improvements or other backward-incompatible changes
2. **Bug Fixes**: Addressing issues in actor logic or runtime
3. **Performance Optimizations**: Improving gas efficiency and execution speed - reducing gas costs is a key focus area
4. **Code Cleanup**: Removing deprecated code, improving maintainability - including actor method deprecation as per FIPs
5. **Testing**: Adding integration tests, scenario tests, or improving test coverage
6. **Documentation**: Improving code documentation and examples
7. **EVM Integration**: Enhancing Ethereum compatibility features
8. **Code Modernization**: Updating to latest Rust features and best practices, or updating / replacing dependencies

### Contribution Workflow

1. **Check existing issues**: Look for relevant issues, especially those labeled `good first issue` or `help wanted`
2. **Create an issue**: For new features or significant changes, create an issue first to discuss the approach
3. **Fork and branch**: Create a feature branch from `master`
4. **Implement changes**: Follow the coding standards and test your changes
5. **Test thoroughly**: Run the full test suite and ensure all checks pass
6. **Submit a pull request**: Follow the PR guidelines below

## Testing

### Running Tests

```bash
# Run all tests
make test

# Run formatting check
make rustfmt

# Run linting
make check

# Build bundles for testing
make bundle
```

### Test Requirements

- All new code must include appropriate unit tests
- Integration tests should be added for cross-actor functionality
- Performance-sensitive changes should include benchmarks
- Test scenarios should cover edge cases and error conditions

### Test Categories

1. **Unit Tests**: Located in each actor's `tests/` directory
2. **Integration Tests**: Located in `integration_tests/src/tests/`
3. **Scenario Tests**: Complex multi-actor workflows
4. **EVM Tests**: Ethereum compatibility testing in `actors/evm/tests/`

## Code Style and Standards

### Rust Guidelines

- Use Rust 2024 edition features
- Follow the project's `rustfmt.toml` configuration
- Address all Clippy warnings (`cargo clippy --all -- -D warnings` - available as `make check`)
- Use explicit error handling with `ActorError` types
- Prefer `cargo-nextest` for test execution

### Actor Development

- Implement the `Actor` trait for new actors
- Use the shared runtime utilities from the `runtime` crate
- Follow existing patterns for state management and CBOR encoding - specifically the Filecoin chain *strictly* uses tuple encoding and some types (such as `BigInt`) have custom encodings that must be explicitly declared
- Ensure gas-efficient implementations - while hard to measure, knowing that gas is measured at the WASM instruction level, efficient code paths tend toward cheaper gas
- Use appropriate error codes and messages
- Include an appropriate amount of logging for future debugging

### Security Considerations

- All actor methods must validate caller permissions - e.g. see `rt.validate_immediate_caller_is()` calls in existing actors
- Input parameters must be thoroughly validated
- State mutations must be atomic and consistent
- Avoid patterns that could lead to reentrancy issues

## Pull Request Guidelines

### PR Title Format

Follow [Conventional Commits](https://www.conventionalcommits.org/) format. If your PR is related to a FIP (Filecoin Improvement Proposal), include the FIP number in the title: `FIP-0XXX: Description`

### PR Description

1. **Clear summary** of what the PR accomplishes
2. **Context** explaining why the change is needed
3. **Testing** information describing how the change was validated
4. **Related issues** using `Closes #XXX` or `Relates to #XXX`
5. **Breaking changes** clearly documented if any

### Review Process

- All PRs require review from core maintainers
- Automated checks must pass (formatting, linting, tests, builds)
- Large changes may require additional review from domain experts
- Draft PRs are encouraged for work-in-progress to get early feedback

### Who merges PRs and when are they merged?

#### Maintainer Approval Process

When maintainers give approval on a PR, they are conveying "this is ready to merge whenever" unless they leave comments indicating otherwise. Maintainers may also approve PRs that have minor cleanup items that would be nice to address but aren't blocking - authors should address these if at all possible.

**Important**: Maintainers generally won't merge PRs for you. This allows authors to:
- Complete any final touchups they may be working on
- Sequence the merge with other related changes
- Maintain control over the timing of their contributions

#### FIP-Related PRs

For PRs implementing FIPs:

- **Standard Process**: We generally wait to merge FIP implementation PRs until the FIP has moved to "Accepted" status.  This includes waiting for FIPs in "Last Call" to move out of "Last Call".
- **Exception for Low-Risk Changes**: If a FIP is uncontroversial and represents pure code cleanup with low risk of needing to revert, we may merge to master before the FIP reaches "Accepted"

This approach minimizes the risk of having to revert changes if FIP requirements change during the review process.

## Release Process

Releases follow a structured process detailed in [RELEASE.md](RELEASE.md).