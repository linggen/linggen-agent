#!/bin/bash
set -euo pipefail

# Sync version to all project files
# Usage: ./scripts/sync-version.sh <version>
#        Version should be without 'v' prefix (e.g., "0.2.2")

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"

VERSION="${1:-}"
if [ -z "$VERSION" ]; then
  echo "Usage: $0 <version>" >&2
  echo "Example: $0 0.2.2" >&2
  exit 1
fi

# Remove 'v' prefix if present
VERSION="${VERSION#v}"

echo "🔄 Syncing version $VERSION to all project files..."

# Update Cargo.toml
if [ -f "$ROOT_DIR/Cargo.toml" ]; then
  if [[ "$OSTYPE" == "darwin"* ]]; then
    sed -i '' "s/^version = \"[^\"]*\"/version = \"$VERSION\"/" "$ROOT_DIR/Cargo.toml"
  else
    sed -i "s/^version = \"[^\"]*\"/version = \"$VERSION\"/" "$ROOT_DIR/Cargo.toml"
  fi
  echo "  ✅ Updated Cargo.toml"

  # Update Cargo.lock
  (cd "$ROOT_DIR" && cargo fetch 2>/dev/null || true)
  echo "  ✅ Updated Cargo.lock"
fi

# Update ui/package.json
if [ -f "$ROOT_DIR/ui/package.json" ]; then
  if [[ "$OSTYPE" == "darwin"* ]]; then
    sed -i '' "s/\"version\": \"[^\"]*\"/\"version\": \"$VERSION\"/" "$ROOT_DIR/ui/package.json"
  else
    sed -i "s/\"version\": \"[^\"]*\"/\"version\": \"$VERSION\"/" "$ROOT_DIR/ui/package.json"
  fi
  echo "  ✅ Updated ui/package.json"
fi

echo "✅ Version sync complete!"
