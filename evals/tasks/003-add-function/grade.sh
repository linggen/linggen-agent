#!/usr/bin/env bash
set -euo pipefail

if [ ! -f src/lib.rs ]; then
    echo "FAIL: src/lib.rs does not exist"
    exit 1
fi

# Check that the multiply function signature exists
if grep -q "pub fn multiply" src/lib.rs; then
    echo "PASS: function 'multiply' found in src/lib.rs"
    exit 0
else
    echo "FAIL: function 'multiply' not found in src/lib.rs"
    exit 1
fi
