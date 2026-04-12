#!/usr/bin/env bash
# Build script for Sage sandbox components.
#
# Builds:
# 1. sage-guest — cross-compiled to aarch64-unknown-linux-musl (runs as PID 1 in VM)
# 2. sandbox-runtime — native binary that hosts the microVM
# 3. Applies macOS HVF entitlements to sandbox-runtime (codesign)
#
# Prerequisites:
#   - rustup target add aarch64-unknown-linux-musl
#   - aarch64-linux-musl-gcc installed (brew install filosottile/musl-cross/musl-cross)
#   - libkrunfw installed at ~/.microsandbox/lib/
#
# Usage:
#   ./scripts/build-sandbox.sh          # release build
#   ./scripts/build-sandbox.sh --debug  # debug build (faster, larger binary)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_ROOT"

# Parse args
PROFILE="release"
CARGO_FLAG="--release"
if [[ "${1:-}" == "--debug" ]]; then
    PROFILE="debug"
    CARGO_FLAG=""
fi

GUEST_TARGET="aarch64-unknown-linux-musl"
GUEST_BIN="target/${GUEST_TARGET}/${PROFILE}/sage-guest"
RUNTIME_BIN="target/${PROFILE}/sandbox-runtime"
ENTITLEMENTS="msb-entitlements.plist"

echo "=== Sage Sandbox Build (${PROFILE}) ==="
echo ""

# Step 1: Verify prerequisites
echo "[1/4] Checking prerequisites..."

if ! rustup target list --installed | grep -q "$GUEST_TARGET"; then
    echo "ERROR: target ${GUEST_TARGET} not installed"
    echo "  Run: rustup target add ${GUEST_TARGET}"
    exit 1
fi

if ! command -v aarch64-linux-musl-gcc &>/dev/null; then
    echo "ERROR: aarch64-linux-musl-gcc not found"
    echo "  Run: brew install filosottile/musl-cross/musl-cross"
    exit 1
fi

KRUNFW_PATH="${HOME}/.microsandbox/lib/libkrunfw.5.dylib"
if [[ ! -f "$KRUNFW_PATH" ]]; then
    echo "WARNING: libkrunfw not found at ${KRUNFW_PATH}"
    echo "  Sandbox won't start without it. Install microsandbox first."
fi

echo "  ✓ Prerequisites OK"

# Step 2: Cross-compile sage-guest
echo ""
echo "[2/4] Building sage-guest (${GUEST_TARGET}, ${PROFILE})..."
cargo build -p sage-guest --target "$GUEST_TARGET" $CARGO_FLAG

if [[ ! -f "$GUEST_BIN" ]]; then
    echo "ERROR: sage-guest binary not found at ${GUEST_BIN}"
    exit 1
fi

GUEST_SIZE=$(stat -f%z "$GUEST_BIN" 2>/dev/null || stat -c%s "$GUEST_BIN" 2>/dev/null)
echo "  ✓ sage-guest built ($(( GUEST_SIZE / 1024 )) KB)"

# Step 3: Build sandbox-runtime (native)
echo ""
echo "[3/4] Building sandbox-runtime (native, ${PROFILE})..."
cargo build -p sage-sandbox --bin sandbox-runtime $CARGO_FLAG

if [[ ! -f "$RUNTIME_BIN" ]]; then
    echo "ERROR: sandbox-runtime binary not found at ${RUNTIME_BIN}"
    exit 1
fi

RUNTIME_SIZE=$(stat -f%z "$RUNTIME_BIN" 2>/dev/null || stat -c%s "$RUNTIME_BIN" 2>/dev/null)
echo "  ✓ sandbox-runtime built ($(( RUNTIME_SIZE / 1024 )) KB)"

# Step 4: macOS HVF entitlements (codesign)
echo ""
echo "[4/4] Applying macOS entitlements..."

if [[ "$(uname)" == "Darwin" ]]; then
    if [[ -f "$ENTITLEMENTS" ]]; then
        codesign --force --sign - --entitlements "$ENTITLEMENTS" "$RUNTIME_BIN"
        echo "  ✓ Entitlements applied (com.apple.security.hypervisor)"
    else
        echo "  WARNING: ${ENTITLEMENTS} not found, skipping codesign"
    fi
else
    echo "  ⊘ Not macOS, skipping entitlements"
fi

# Summary
echo ""
echo "=== Build Complete ==="
echo "  sage-guest:       ${GUEST_BIN}"
echo "  sandbox-runtime:  ${RUNTIME_BIN}"
echo ""
echo "To run:"
echo "  sage run --config configs/coding-assistant.yaml --message '...'"
