# Contributing to Filecoin Built-in Actors

Welcome to the Filecoin built-in actors repository! This codebase serves as the canonical implementation of on-chain actors that power the Filecoin network and is a common good across all Filecoin client implementations.

## Table of Contents

- [Getting Started](#getting-started)
- [Development Setup](#development-setup)
- [Making Contributions](#making-contributions)
- [Testing](#testing)
- [Code Style and Standards](#code-style-and-standards)
- [Pull Request Guidelines](#pull-request-guidelines)
- [Issue Guidelines](#issue-guidelines)
- [Release Process](#release-process)
- [Getting Help](#getting-help)

## Getting Started

### Prerequisites

- **Rust**: This project uses Rust 2024 edition. Install via [rustup](https://rustup.rs/)
- **Make**: Required for build automation
- **Docker**: Required for reproducible builds
- **Git**: For version control
- **GitHub CLI (gh)**: Recommended for managing issues and PRs

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

1. **FIP Implementation**: Implementing Filecoin Improvement Proposals
2. **Bug Fixes**: Addressing issues in actor logic or runtime
3. **Performance Optimizations**: Improving gas efficiency and execution speed
4. **Code Cleanup**: Removing deprecated code, improving maintainability
5. **Testing**: Adding integration tests, scenario tests, or improving test coverage
6. **Documentation**: Improving code documentation and examples

### Contribution Workflow

1. **Check existing issues**: Look for relevant issues, especially those labeled `good first issue` or `help wanted`
2. **Create an issue**: For new features or significant changes, create an issue first to discuss the approach
3. **Fork and branch**: Create a feature branch from `master`
4. **Implement changes**: Follow the coding standards and test your changes
5. **Test thoroughly**: Run the full test suite and ensure all checks pass
6. **Submit a pull request**: Follow the PR guidelines below

### Common Contribution Areas

- **FIP Implementation**: Many contributions involve implementing protocol improvements
- **Actor Method Deprecation**: Removing deprecated methods as per FIPs
- **Performance Optimization**: Reducing gas costs and improving execution efficiency
- **EVM Integration**: Enhancing Ethereum compatibility features
- **Code Modernization**: Updating to latest Rust features and best practices

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
- Address all Clippy warnings (`cargo clippy --all -- -D warnings`)
- Use explicit error handling with `ActorError` types
- Prefer `cargo-nextest` for test execution

### Actor Development

- Implement the `Actor` trait for new actors
- Use the shared runtime utilities from the `runtime` crate
- Follow existing patterns for state management and CBOR encoding
- Ensure gas-efficient implementations
- Use appropriate error codes and messages

### Security Considerations

- All actor methods must validate caller permissions
- Input parameters must be thoroughly validated
- State mutations must be atomic and consistent
- Avoid patterns that could lead to reentrancy issues

## Pull Request Guidelines

### PR Title Format

Use descriptive prefixes:
- `feat:` - New features
- `fix:` - Bug fixes  
- `chore:` - Maintenance tasks
- `test:` - Testing improvements
- `opt:` - Performance optimizations
- `build:` - Build system changes

Include FIP numbers when applicable: `FIP-0XXX: Description`

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

## Issue Guidelines

### Reporting Bugs

Include:
- Clear reproduction steps
- Expected vs actual behavior
- Environment details (Rust version, network, etc.)
- Relevant error messages or logs

### Feature Requests

- Check if a FIP exists for the feature
- Describe the use case and motivation
- Consider backwards compatibility implications
- Provide implementation suggestions if possible

### Labels

- `good first issue`: Suitable for new contributors
- `help wanted`: Community contributions welcome
- `cleanup`: Code maintenance and cleanup
- `enhancement`: New features or improvements
- `bug`: Issues with existing functionality
- `testing`: Test-related improvements

## Release Process

Releases follow a structured process detailed in [RELEASE.md](RELEASE.md):

1. Version updates are made via PR to `Cargo.toml`
2. Automated workflows handle bundle building and asset uploads
3. Git tags are created when releases are published
4. Multiple network bundles are built for each release

## Getting Help

### Resources

- **Documentation**: Check the [README](README.md) and inline code documentation
- **Examples**: See the `examples/` directory for usage patterns
- **Integration Tests**: Review `integration_tests/` for complex scenarios
- **FIPs**: Filecoin Improvement Proposals at [github.com/filecoin-project/FIPs](https://github.com/filecoin-project/FIPs)

### Communication

- **GitHub Issues**: For bug reports and feature requests
- **GitHub Discussions**: For general questions and community discussion
- **Filecoin Slack**: Join the [Filecoin Slack](https://filecoin.io/slack) for real-time discussion

### Finding Contribution Opportunities

- Browse issues labeled `good first issue` for beginner-friendly tasks
- Check `help wanted` issues for community contribution opportunities
- Look for `cleanup` labeled issues for code maintenance tasks
- Review open FIPs that need implementation

## Code of Conduct

This project follows the Filecoin community standards. Be respectful, inclusive, and constructive in all interactions.

## License

This project is dual-licensed under [MIT](LICENSE-MIT) and [Apache 2.0](LICENSE-APACHE) licenses. By contributing, you agree to license your contributions under the same terms.

---

Thank you for contributing to the Filecoin built-in actors! Your contributions help secure and improve the Filecoin network for everyone.