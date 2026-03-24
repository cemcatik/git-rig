# git-rig development recipes

# Run all checks (fmt + clippy + deny + test)
check: fmt clippy deny test

# Check formatting
fmt:
    cargo fmt --check

# Run clippy lints
clippy:
    cargo clippy --all-targets -- -D warnings

# Run cargo-deny (license + advisory audit). Skips if cargo-deny is not installed.
deny:
    @command -v cargo-deny >/dev/null 2>&1 && cargo deny check || echo "warn: cargo-deny not installed, skipping (install with: cargo install cargo-deny)"

# Run all tests
test:
    cargo test

# Run unit tests only (manifest ops, workspace resolution)
test-unit:
    cargo test --bin git-rig

# Run integration tests (git operations against real repos)
test-integration:
    cargo test --test git_test

# Run E2E tests (full CLI commands via assert_cmd)
test-e2e:
    cargo test --test cli_test

# Run coverage and generate lcov report
coverage:
    cargo llvm-cov --lcov --output-path lcov.info

# Install to ~/.cargo/bin/git-rig
install:
    cargo install --path .
