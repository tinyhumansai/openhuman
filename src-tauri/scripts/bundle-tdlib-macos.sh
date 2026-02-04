#!/bin/bash
# Script to bundle TDLib dylib and its dependencies into macOS app bundle
# This fixes the "Library not loaded: @rpath/libtdjson.1.8.29.dylib" error

set -e

TDLIB_VERSION="1.8.29"
DYLIB_NAME="libtdjson.${TDLIB_VERSION}.dylib"

# Determine build type (debug or release)
BUILD_TYPE="${1:-release}"
# Optional: specific target (e.g., aarch64-apple-darwin, x86_64-apple-darwin)
TARGET="${2:-}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
TAURI_DIR="$(dirname "$SCRIPT_DIR")"

# Determine target directory (handles cross-compilation targets)
if [ -n "$TARGET" ]; then
    TARGET_DIR="${TAURI_DIR}/target/${TARGET}/${BUILD_TYPE}"
else
    TARGET_DIR="${TAURI_DIR}/target/${BUILD_TYPE}"
fi

# Fallback: if target-specific dir doesn't exist, try without target
if [ ! -d "$TARGET_DIR" ]; then
    TARGET_DIR="${TAURI_DIR}/target/${BUILD_TYPE}"
fi

echo "=== TDLib macOS Bundler ==="
echo "Build type: ${BUILD_TYPE}"
echo "Target: ${TARGET:-default}"
echo "Target dir: ${TARGET_DIR}"

# Find the app bundle - check multiple possible locations
APP_BUNDLE=""
for search_dir in \
    "${TAURI_DIR}/target/${TARGET}/${BUILD_TYPE}/bundle/macos" \
    "${TAURI_DIR}/target/${BUILD_TYPE}/bundle/macos" \
    "${TAURI_DIR}/target/release/bundle/macos" \
    "${TAURI_DIR}/target/debug/bundle/macos"; do
    if [ -d "$search_dir" ]; then
        found=$(find "$search_dir" -name "*.app" -type d 2>/dev/null | head -1)
        if [ -n "$found" ]; then
            APP_BUNDLE="$found"
            break
        fi
    fi
done

if [ -z "$APP_BUNDLE" ]; then
    echo "Warning: No .app bundle found"
    echo "Searched in:"
    echo "  - ${TAURI_DIR}/target/${TARGET}/${BUILD_TYPE}/bundle/macos"
    echo "  - ${TAURI_DIR}/target/${BUILD_TYPE}/bundle/macos"
    echo "This script should be run after 'tauri build'"
    exit 0
fi

echo "Found app bundle: ${APP_BUNDLE}"

# Create Frameworks directory
FRAMEWORKS_DIR="${APP_BUNDLE}/Contents/Frameworks"
mkdir -p "$FRAMEWORKS_DIR"
echo "Created Frameworks directory: ${FRAMEWORKS_DIR}"

# Find the TDLib dylib in build artifacts - search in multiple locations
DYLIB_PATH=""
for search_dir in \
    "${TAURI_DIR}/target/${TARGET}/${BUILD_TYPE}/build" \
    "${TAURI_DIR}/target/${BUILD_TYPE}/build" \
    "${TAURI_DIR}/target/release/build" \
    "${TAURI_DIR}/target/debug/build"; do
    if [ -d "$search_dir" ]; then
        found=$(find "$search_dir" -name "${DYLIB_NAME}" -type f 2>/dev/null | head -1)
        if [ -n "$found" ]; then
            DYLIB_PATH="$found"
            break
        fi
    fi
done

if [ -z "$DYLIB_PATH" ]; then
    echo "Error: Could not find ${DYLIB_NAME} in build artifacts"
    echo "Searched in target/*/build directories"
    echo "Make sure TDLib was built correctly."
    exit 1
fi

echo "Found TDLib dylib: ${DYLIB_PATH}"

# Copy the dylib to Frameworks
cp "$DYLIB_PATH" "${FRAMEWORKS_DIR}/"
echo "Copied ${DYLIB_NAME} to Frameworks"

# Also copy the symlink version if it exists
DYLIB_SYMLINK="${DYLIB_PATH%/*}/libtdjson.dylib"
if [ -f "$DYLIB_SYMLINK" ] || [ -L "$DYLIB_SYMLINK" ]; then
    cp "$DYLIB_SYMLINK" "${FRAMEWORKS_DIR}/" 2>/dev/null || true
fi

# Find the main binary
BINARY_NAME=$(basename "${APP_BUNDLE}" .app)
BINARY_PATH="${APP_BUNDLE}/Contents/MacOS/${BINARY_NAME}"

if [ ! -f "$BINARY_PATH" ]; then
    echo "Error: Binary not found at ${BINARY_PATH}"
    exit 1
fi

echo "Found binary: ${BINARY_PATH}"

# Bundled dylib path
BUNDLED_DYLIB="${FRAMEWORKS_DIR}/${DYLIB_NAME}"

# Function to bundle a dependency and fix references
bundle_dependency() {
    local dep_path="$1"
    local dep_name=$(basename "$dep_path")

    if [ ! -f "$dep_path" ]; then
        echo "Warning: Dependency not found: $dep_path"
        return 1
    fi

    # Copy to Frameworks if not already there
    if [ ! -f "${FRAMEWORKS_DIR}/${dep_name}" ]; then
        echo "Bundling dependency: ${dep_name}"
        cp "$dep_path" "${FRAMEWORKS_DIR}/"
        chmod 755 "${FRAMEWORKS_DIR}/${dep_name}"

        # Change the dependency's install name to use @rpath
        install_name_tool -id "@rpath/${dep_name}" "${FRAMEWORKS_DIR}/${dep_name}"
    fi

    return 0
}

# Function to fix references in a library
fix_references() {
    local lib_path="$1"
    local lib_name=$(basename "$lib_path")

    echo "Fixing references in: ${lib_name}"

    # Get all dependencies
    local deps=$(otool -L "$lib_path" | tail -n +2 | awk '{print $1}')

    for dep in $deps; do
        # Skip system libraries (they're fine as-is)
        if [[ "$dep" == /usr/lib/* ]] || [[ "$dep" == /System/* ]] || [[ "$dep" == "@rpath/"* ]] || [[ "$dep" == "@executable_path/"* ]]; then
            continue
        fi

        local dep_name=$(basename "$dep")

        # Bundle the dependency if it's from Homebrew or a non-system path
        if [[ "$dep" == /opt/homebrew/* ]] || [[ "$dep" == /usr/local/* ]]; then
            if bundle_dependency "$dep"; then
                # Update the reference to use @rpath
                install_name_tool -change "$dep" "@rpath/${dep_name}" "$lib_path"
                echo "  Fixed reference: ${dep_name}"
            fi
        fi
    done
}

# Fix the TDLib dylib's install name
echo ""
echo "Fixing dylib install name..."
install_name_tool -id "@rpath/${DYLIB_NAME}" "$BUNDLED_DYLIB"

# Bundle OpenSSL and other dependencies from TDLib
echo ""
echo "Bundling TDLib dependencies..."
fix_references "$BUNDLED_DYLIB"

# Fix any cross-references between bundled dependencies
echo ""
echo "Fixing cross-references between dependencies..."
for dylib in "${FRAMEWORKS_DIR}"/*.dylib; do
    if [ -f "$dylib" ]; then
        fix_references "$dylib"
    fi
done

# Fix rpaths in the binary
echo ""
echo "Fixing binary rpaths..."

# Get all current rpaths
CURRENT_RPATHS=$(otool -l "$BINARY_PATH" | grep -A2 "cmd LC_RPATH" | grep "path " | awk '{print $2}')

echo "Current rpaths:"
echo "$CURRENT_RPATHS"

# Remove all rpaths that point to build directories (they won't exist at runtime)
for rpath in $CURRENT_RPATHS; do
    # Remove rpaths containing /build/, /target/, or absolute paths that aren't @executable_path
    if [[ "$rpath" == *"/build/"* ]] || [[ "$rpath" == *"/target/"* ]] || [[ "$rpath" != @* ]]; then
        echo "Removing build-time rpath: $rpath"
        install_name_tool -delete_rpath "$rpath" "$BINARY_PATH" 2>/dev/null || true
    fi
done

# Delete our target rpath first if it exists (to avoid "already exists" error)
install_name_tool -delete_rpath "@executable_path/../Frameworks" "$BINARY_PATH" 2>/dev/null || true

# Add the correct rpath for the bundled Frameworks
echo "Adding rpath: @executable_path/../Frameworks"
install_name_tool -add_rpath "@executable_path/../Frameworks" "$BINARY_PATH"

# Verify the changes
echo ""
echo "=== Verification ==="
echo ""
echo "Bundled libraries:"
ls -la "${FRAMEWORKS_DIR}/"

echo ""
echo "TDLib dylib dependencies (should all be @rpath or system libs):"
otool -L "$BUNDLED_DYLIB"

echo ""
echo "Binary library dependencies:"
otool -L "$BINARY_PATH" | grep -i tdjson || echo "No tdjson reference found"

echo ""
echo "Binary rpaths:"
otool -l "$BINARY_PATH" | grep -A3 "cmd LC_RPATH" | head -8 || true

echo ""
echo "=== TDLib bundling complete ==="
echo "The app bundle at ${APP_BUNDLE} now includes TDLib and its dependencies."
