#!/bin/bash
set -euo pipefail

# Release orchestrator script for Linggen Agent
# Usage: ./scripts/release.sh <version> [--draft] [--platform mac|linux]
#
# Default platform is the current host (no cross-build):
#   - macOS host  → mac
#   - Linux host  → linux (multi-arch: x86_64 + aarch64)

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
source "$ROOT_DIR/scripts/lib-common.sh"

REPO="linggen/linggen"
VERSION=""
KEEP_DRAFT=false
PLATFORM=""
PASS_ARGS=()

# Parse arguments
while [[ $# -gt 0 ]]; do
  case "$1" in
    --draft)
      KEEP_DRAFT=true
      shift ;;
    --platform)
      PLATFORM="${2:-}"
      PASS_ARGS+=("--platform" "$PLATFORM")
      shift 2 ;;
    --platform=*)
      PLATFORM="${1#--platform=}"
      PASS_ARGS+=("$1")
      shift ;;
    *)
      if [ -z "$VERSION" ]; then
        VERSION="$1"
      fi
      shift ;;
  esac
done

if [ -z "$VERSION" ]; then
  echo "Usage: $0 <version> [--draft] [--platform mac|linux]" >&2
  exit 1
fi

OS_LOWER="$(uname -s | tr '[:upper:]' '[:lower:]')"
HOST_PLATFORM="$([ "$OS_LOWER" = "darwin" ] && echo mac || echo linux)"
PLATFORM="${PLATFORM:-$HOST_PLATFORM}"

case "$PLATFORM" in
  mac|linux) ;;
  *)
    echo "Error: --platform must be 'mac' or 'linux' (got '$PLATFORM')" >&2
    exit 1 ;;
esac

VERSION_NUM="${VERSION#v}"
DIST_DIR="$ROOT_DIR/dist"

# Step 1: Build everything
echo "📦 Step 1: Building all artifacts..."
"$ROOT_DIR/scripts/build.sh" "$VERSION" ${PASS_ARGS[@]+"${PASS_ARGS[@]}"}

SLUG=$(detect_platform)

# Step 2: Create GitHub Release
echo ""
echo "🚀 Step 2: Creating GitHub Release..."
if gh release view "$VERSION" --repo "$REPO" &>/dev/null; then
  echo "✅ Release ${VERSION} already exists"
else
  gh release create "$VERSION" \
    --repo "$REPO" \
    --title "Linggen ${VERSION}" \
    --notes "Release ${VERSION}" \
    --draft
  echo "✅ Created draft release ${VERSION}"
fi

# Step 3: Upload Artifacts
echo ""
echo "📤 Step 3: Uploading artifacts..."

delete_asset() {
  local name="$1"
  gh release delete-asset "$VERSION" "$name" --repo "$REPO" --yes 2>/dev/null || true
}

TARBALL="$DIST_DIR/ling-${SLUG}.tar.gz"
HAS_MAC_TARBALL=false
HAS_LINUX_DIR=false
[ "$PLATFORM" = "mac" ] && [ -f "$TARBALL" ] && HAS_MAC_TARBALL=true
[ -d "$DIST_DIR/linux" ] && HAS_LINUX_DIR=true

if [ "$HAS_MAC_TARBALL" = "false" ] && [ "$HAS_LINUX_DIR" = "false" ]; then
  echo "Error: no artifacts to upload — did the build step produce anything?" >&2
  echo "Looked for: $TARBALL and $DIST_DIR/linux/" >&2
  exit 1
fi

# ling binary tarball (mac platform only — linux artifacts live under dist/linux/)
if [ "$HAS_MAC_TARBALL" = "true" ]; then
  echo "  Uploading: $(basename "$TARBALL")"
  delete_asset "$(basename "$TARBALL")"
  gh release upload "$VERSION" "$TARBALL" --repo "$REPO"
fi

# Linux Artifacts (multi-arch from Docker)
if [ "$HAS_LINUX_DIR" = "true" ]; then
  echo "  Uploading Linux artifacts..."
  for file in "$DIST_DIR/linux"/*; do
    if [ -f "$file" ]; then
      echo "    Uploading: $(basename "$file")"
      delete_asset "$(basename "$file")"
      gh release upload "$VERSION" "$file" --repo "$REPO"
    fi
  done
fi

# Step 4: Generate and Upload Manifest
echo ""
echo "📄 Step 4: Generating and uploading manifest..."
BASE_URL="https://github.com/${REPO}/releases/download/${VERSION}"

# Build assets array from the release's actual asset list, not just local dist/.
# This makes split-host workflows additive: running with --platform mac on a Mac
# and later --platform linux on a Linux box keeps both sets of entries in the
# manifest, instead of the second run overwriting the first.
ASSETS=$(gh release view "$VERSION" --repo "$REPO" --json assets \
  | jq --arg base "$BASE_URL" \
      '[.assets[]
         | select(.name | test("^ling-.*\\.tar\\.gz$"))
         | {name: (.name | sub("\\.tar\\.gz$"; "")),
            url: ($base + "/" + .name)}]')

MANIFEST_JSON=$(jq -n \
  --arg version "${VERSION_NUM}" \
  --argjson assets "$ASSETS" \
  '{version: $version, assets: $assets}')

echo "$MANIFEST_JSON" > "$DIST_DIR/manifest.json"

delete_asset "manifest.json"
gh release upload "$VERSION" "$DIST_DIR/manifest.json" --repo "$REPO"

# Step 5: Finalize
if [ "$KEEP_DRAFT" = "true" ]; then
  echo "⚠️  Draft release ${VERSION} created."
else
  echo "🚀 Publishing release..."
  gh release edit "$VERSION" --draft=false --latest --repo "$REPO"
  echo "✅ Release ${VERSION} published!"
  echo "curl -fsSL https://linggen.dev/install.sh | bash"
fi
