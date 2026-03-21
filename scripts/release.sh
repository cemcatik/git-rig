#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:-}"

if [ -z "$VERSION" ]; then
  echo "Usage: scripts/release.sh <version>"
  echo "Example: scripts/release.sh 0.2.0"
  exit 1
fi

# Strip leading 'v' if provided
VERSION="${VERSION#v}"

# Validate semver format
if ! [[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$ ]]; then
  echo "Error: '$VERSION' is not a valid semver version (expected X.Y.Z)"
  exit 1
fi

TAG="v$VERSION"

# Check for clean working tree
if ! git diff --quiet || ! git diff --cached --quiet; then
  echo "Error: working tree is dirty — commit or stash changes first"
  exit 1
fi

# Check tag doesn't already exist
if git rev-parse "$TAG" >/dev/null 2>&1; then
  echo "Error: tag '$TAG' already exists"
  exit 1
fi

# Bump version in Cargo.toml
CURRENT=$(grep '^version = ' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
echo "Bumping version: $CURRENT -> $VERSION"

sed -i '' "s/^version = \"$CURRENT\"/version = \"$VERSION\"/" Cargo.toml

# Update Cargo.lock
cargo check --quiet

# Commit, tag, push
git add Cargo.toml Cargo.lock
git commit -m "chore: release v$VERSION"
git tag "$TAG"
git push --atomic origin HEAD "$TAG"

echo ""
echo "Released $TAG — release workflow will start shortly"
echo "  https://github.com/cemcatik/git-rig/actions"
