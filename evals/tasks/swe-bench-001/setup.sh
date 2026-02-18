#!/usr/bin/env bash
# Setup script for SWE-bench sympy__sympy-21379
# Clones the sympy repo, checks out the base commit, and installs in editable mode.
set -euo pipefail

REPO="https://github.com/sympy/sympy.git"
BASE_COMMIT="624217179aaf8d094e6ff75b7493ad1ee47599b0"

echo "==> Cloning sympy/sympy (shallow)..."
# Clone into a temp subdirectory, then move .git into $EVAL_WORKSPACE
tmpclone=$(mktemp -d)
git clone --no-checkout "$REPO" "$tmpclone/sympy" 2>&1

echo "==> Moving .git into workspace..."
mv "$tmpclone/sympy/.git" "$EVAL_WORKSPACE/.git"
rm -rf "$tmpclone"

echo "==> Checking out base commit $BASE_COMMIT..."
cd "$EVAL_WORKSPACE"
git checkout "$BASE_COMMIT" -- . 2>&1

echo "==> Installing sympy in editable mode..."
pip3 install -e . --quiet 2>&1

echo "==> Setup complete."
