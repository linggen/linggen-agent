#!/bin/bash
# Ensure linggen CLI is installed, then start server and check status.
set -euo pipefail

if command -v linggen >/dev/null 2>&1; then
    echo "✓ linggen CLI found: $(linggen --version 2>/dev/null || echo "version unknown")"
else
    echo "Linggen CLI not found. Installing..."
    curl -fsSL https://linggen.dev/install-cli.sh | bash
fi

echo ""
echo "Running: linggen doctor"
if ! linggen doctor; then
    echo "Warning: linggen doctor reported issues."
fi

echo ""
echo "Starting Linggen server..."
linggen start

echo ""
echo "Running: linggen check"
linggen check
