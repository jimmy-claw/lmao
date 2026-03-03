#!/usr/bin/env bash
# Package the module as a .lgx file for Logos Core
# .lgx = tar.gz with metadata.json + shared libs + assets
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(dirname "$SCRIPT_DIR")"
BUILD_DIR="${ROOT}/module/build"
OUT_DIR="${ROOT}/dist"
VERSION=$(jq -r .version "${ROOT}/module/metadata.json")
PACKAGE_NAME="logos-messaging-a2a-${VERSION}"

echo "==> Packaging ${PACKAGE_NAME}.lgx"

# Qt UI module is optional — skip with a warning if not built
HAS_UI=true
if [ ! -f "${BUILD_DIR}/libmessaging_a2a_ui.so" ]; then
    echo "WARN: libmessaging_a2a_ui.so not found — packaging without Qt UI module."
    echo "      To include it, build with: cd module && mkdir -p build && cd build && cmake .. [flags] && make -j\$(nproc)"
    HAS_UI=false
fi

FFI_LIB="${ROOT}/target/release/liblmao_ffi.so"
if [ ! -f "$FFI_LIB" ]; then
    echo "ERROR: liblmao_ffi.so not found in target/release/. Build with:"
    echo "  cargo build --release -p lmao-ffi"
    exit 1
fi

# Stage files
STAGING=$(mktemp -d)
trap "rm -rf $STAGING" EXIT

mkdir -p "${STAGING}/${PACKAGE_NAME}/lib"
mkdir -p "${STAGING}/${PACKAGE_NAME}/qml"
mkdir -p "${STAGING}/${PACKAGE_NAME}/icons"

if [ "$HAS_UI" = true ]; then
    cp "${BUILD_DIR}/libmessaging_a2a_ui.so" "${STAGING}/${PACKAGE_NAME}/lib/"
fi
cp "$FFI_LIB"                            "${STAGING}/${PACKAGE_NAME}/lib/"
cp "${ROOT}/module/metadata.json"         "${STAGING}/${PACKAGE_NAME}/"
cp "${ROOT}/module/qml/"*.qml            "${STAGING}/${PACKAGE_NAME}/qml/" 2>/dev/null || true
cp "${ROOT}/module/icons/"*              "${STAGING}/${PACKAGE_NAME}/icons/" 2>/dev/null || true

# Create .lgx (tar.gz)
mkdir -p "$OUT_DIR"
tar -czf "${OUT_DIR}/${PACKAGE_NAME}.lgx" -C "$STAGING" "$PACKAGE_NAME"

echo "==> Created ${OUT_DIR}/${PACKAGE_NAME}.lgx"
echo "    Contents:"
tar -tzf "${OUT_DIR}/${PACKAGE_NAME}.lgx"
