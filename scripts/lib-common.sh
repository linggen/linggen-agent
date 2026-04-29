#!/bin/bash
# Shared library for release and packaging scripts

# Detect platform and set SLUG
detect_platform() {
  local OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
  local ARCH="$(uname -m)"
  local SLUG=""

  case "$OS" in
    darwin)
      case "$ARCH" in
        arm64|aarch64) SLUG="macos-aarch64" ;;
        x86_64|amd64)  SLUG="macos-x86_64" ;;
        *) echo "Unsupported macOS arch: $ARCH" >&2; return 1 ;;
      esac
      ;;
    linux)
      case "$ARCH" in
        x86_64|amd64) SLUG="linux-x86_64" ;;
        arm64|aarch64) SLUG="linux-aarch64" ;;
        *) echo "Unsupported Linux arch: $ARCH" >&2; return 1 ;;
      esac
      ;;
    *)
      echo "Unsupported OS: $OS" >&2; return 1
      ;;
  esac
  echo "$SLUG"
}
