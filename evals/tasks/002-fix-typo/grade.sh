#!/usr/bin/env bash
set -euo pipefail

if [ ! -f src/lib.rs ]; then
    echo "FAIL: src/lib.rs does not exist"
    exit 1
fi

# The typo should be gone
if grep -q "calulate_sum" src/lib.rs; then
    echo "FAIL: typo 'calulate_sum' still present in src/lib.rs"
    exit 1
fi

# The correct name should be present
if grep -q "calculate_sum" src/lib.rs; then
    echo "PASS: typo fixed, 'calculate_sum' found"
    exit 0
else
    echo "FAIL: 'calculate_sum' not found in src/lib.rs"
    exit 1
fi
