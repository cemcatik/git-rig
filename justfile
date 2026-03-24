# git-rig development recipes

# Run all checks (fmt + clippy + test). Run `just deny` separately if cargo-deny is installed.
check: fmt clippy test

# Check formatting
fmt:
    cargo fmt --check

# Run clippy lints
clippy:
    cargo clippy --all-targets -- -D warnings

# Run cargo-deny (license + advisory audit)
deny:
    cargo deny check

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

# Install git hooks
install-hooks:
    @echo "Installing pre-commit hook..."
    @cp hooks/pre-commit .git/hooks/pre-commit
    @chmod +x .git/hooks/pre-commit
    @echo "Done."
