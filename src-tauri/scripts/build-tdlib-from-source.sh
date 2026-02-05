#!/usr/bin/env bash
#
# Build TDLib v1.8.29 from source with MACOSX_DEPLOYMENT_TARGET=10.15
# OpenSSL 3.x is statically linked so there are NO external OpenSSL deps.
#
# Usage:
#   cd src-tauri
#   ./scripts/build-tdlib-from-source.sh [arm64|x86_64]
#
# Output: src-tauri/tdlib-local/  (lib/ + include/)
# Time:   5-15 minutes on first run

set -euo pipefail

TDLIB_VERSION="1.8.29"
TDLIB_COMMIT="af69dd4397b6dc1bf23ba0fd0bf429fcba6454f6"
OPENSSL_VERSION="3.4.1"

export MACOSX_DEPLOYMENT_TARGET="10.15"

# --- Resolve paths relative to src-tauri/ ---
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
SRC_TAURI_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# --- Detect or override architecture ---
ARCH="${1:-$(uname -m)}"
case "$ARCH" in
    arm64|aarch64) ARCH="arm64" ;;
    x86_64)        ARCH="x86_64" ;;
    *)
        echo "Error: unsupported architecture '$ARCH'. Use arm64 or x86_64."
        exit 1
        ;;
esac
echo "==> Building for architecture: $ARCH"
echo "==> Deployment target: macOS $MACOSX_DEPLOYMENT_TARGET"

# Arch-specific build and install dirs (so arm64 and x86_64 don't clobber each other)
BUILD_DIR="${SRC_TAURI_DIR}/tdlib-build/${ARCH}"
INSTALL_DIR="${SRC_TAURI_DIR}/tdlib-local/${ARCH}"

COMMON_FLAGS="-arch $ARCH -mmacosx-version-min=$MACOSX_DEPLOYMENT_TARGET"

# --- Check prerequisites ---
for cmd in cmake gperf make perl; do
    if ! command -v $cmd &>/dev/null; then
        echo "Error: '$cmd' not found. Install via: brew install $cmd"
        exit 1
    fi
done

mkdir -p "$BUILD_DIR"

# ============================================================
# 1. Build OpenSSL 3.x (static only)
# ============================================================
OPENSSL_SRC="${BUILD_DIR}/openssl-${OPENSSL_VERSION}"
OPENSSL_INSTALL="${BUILD_DIR}/openssl-install"

if [ -f "${OPENSSL_INSTALL}/lib/libssl.a" ]; then
    echo "==> OpenSSL already built, skipping (delete ${OPENSSL_INSTALL} to rebuild)"
else
    echo "==> Downloading OpenSSL ${OPENSSL_VERSION}..."
    cd "$BUILD_DIR"
    if [ ! -d "$OPENSSL_SRC" ]; then
        curl -sSL "https://github.com/openssl/openssl/releases/download/openssl-${OPENSSL_VERSION}/openssl-${OPENSSL_VERSION}.tar.gz" -o openssl.tar.gz
        tar xzf openssl.tar.gz
        rm openssl.tar.gz
    fi

    echo "==> Building OpenSSL (static, no-shared)..."
    cd "$OPENSSL_SRC"

    # Map arch to OpenSSL target
    if [ "$ARCH" = "arm64" ]; then
        OPENSSL_TARGET="darwin64-arm64-cc"
    else
        OPENSSL_TARGET="darwin64-x86_64-cc"
    fi

    ./Configure "$OPENSSL_TARGET" \
        no-shared \
        no-tests \
        --prefix="$OPENSSL_INSTALL" \
        -mmacosx-version-min=$MACOSX_DEPLOYMENT_TARGET

    make -j"$(sysctl -n hw.ncpu)"
    make install_sw
    echo "==> OpenSSL installed to ${OPENSSL_INSTALL}"
fi

# ============================================================
# 2. Build TDLib from source
# ============================================================
TDLIB_SRC="${BUILD_DIR}/td"
TDLIB_BUILD="${BUILD_DIR}/td-build"

if [ -f "${INSTALL_DIR}/lib/libtdjson.${TDLIB_VERSION}.dylib" ]; then
    echo "==> TDLib already built, skipping (delete ${INSTALL_DIR} to rebuild)"
else
    echo "==> Downloading TDLib ${TDLIB_VERSION} (commit ${TDLIB_COMMIT})..."
    cd "$BUILD_DIR"
    if [ ! -d "$TDLIB_SRC" ]; then
        git clone https://github.com/tdlib/td.git
        cd td && git checkout "$TDLIB_COMMIT" && cd ..
    fi

    echo "==> Building TDLib..."
    rm -rf "$TDLIB_BUILD"
    mkdir -p "$TDLIB_BUILD"
    cd "$TDLIB_BUILD"

    # CMAKE_OSX_ARCHITECTURES ensures all C/C++ code targets the right arch
    cmake "$TDLIB_SRC" \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_INSTALL_PREFIX="$INSTALL_DIR" \
        -DCMAKE_OSX_DEPLOYMENT_TARGET="$MACOSX_DEPLOYMENT_TARGET" \
        -DCMAKE_OSX_ARCHITECTURES="$ARCH" \
        -DCMAKE_C_FLAGS="$COMMON_FLAGS" \
        -DCMAKE_CXX_FLAGS="$COMMON_FLAGS" \
        -DOPENSSL_ROOT_DIR="$OPENSSL_INSTALL" \
        -DOPENSSL_USE_STATIC_LIBS=ON \
        -DOPENSSL_CRYPTO_LIBRARY="$OPENSSL_INSTALL/lib/libcrypto.a" \
        -DOPENSSL_SSL_LIBRARY="$OPENSSL_INSTALL/lib/libssl.a" \
        -DOPENSSL_INCLUDE_DIR="$OPENSSL_INSTALL/include" \
        -DTD_ENABLE_JNI=OFF

    cmake --build . --target tdjson -j"$(sysctl -n hw.ncpu)"
    cmake --install .

    echo "==> TDLib installed to ${INSTALL_DIR}"
fi

# ============================================================
# 3. Copy to tdlib-prebuilt/ for git commit
# ============================================================
PREBUILT_DIR="${SRC_TAURI_DIR}/tdlib-prebuilt/macos-${ARCH}"
mkdir -p "$PREBUILT_DIR"
cp "${INSTALL_DIR}/lib/libtdjson.${TDLIB_VERSION}.dylib" "$PREBUILT_DIR/"
echo "==> Copied dylib to ${PREBUILT_DIR}/ (commit this to git)"

# ============================================================
# 4. Verify the build
# ============================================================
DYLIB="${INSTALL_DIR}/lib/libtdjson.${TDLIB_VERSION}.dylib"
if [ ! -f "$DYLIB" ]; then
    echo "Error: expected dylib not found at ${DYLIB}"
    exit 1
fi

echo ""
echo "==> Verification:"

# Check deployment target
MINOS=$(otool -l "$DYLIB" | grep -A2 "minos" | head -3)
echo "    Min OS version info:"
echo "$MINOS" | sed 's/^/      /'

# Check for OpenSSL references (should be NONE)
OPENSSL_REFS=$(otool -L "$DYLIB" | grep -i "ssl\|crypto" || true)
if [ -n "$OPENSSL_REFS" ]; then
    echo "    WARNING: Found external OpenSSL references (should be none):"
    echo "$OPENSSL_REFS" | sed 's/^/      /'
else
    echo "    No external OpenSSL references (statically linked)"
fi

# Show all dylib deps
echo "    Dependencies:"
otool -L "$DYLIB" | tail -n +2 | sed 's/^/      /'

echo ""
echo "==> Done! TDLib ${TDLIB_VERSION} built successfully for ${ARCH}."
echo "    Install dir: ${INSTALL_DIR}"
echo ""
echo "    Next: run 'yarn tauri build --debug --bundles app' to build the app."
