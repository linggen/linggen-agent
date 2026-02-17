#!/usr/bin/env bash
set -euo pipefail

if [ ! -f hello.txt ]; then
    echo "FAIL: hello.txt does not exist"
    exit 1
fi

content=$(cat hello.txt)
if [ "$content" = "Hello, World!" ]; then
    echo "PASS: hello.txt has correct content"
    exit 0
else
    echo "FAIL: expected 'Hello, World!' but got '$content'"
    exit 1
fi
