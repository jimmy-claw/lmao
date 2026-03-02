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

# Ensure build artifacts exist
if [ ! -f "${BUILD_DIR}/libmessaging_a2a_ui.so" ]; then
    echo "ERROR: libmessaging_a2a_ui.so not found. Build the module first."
    echo "  cargo build --release -p waku-a2a-ffi"
    echo "  cd module && mkdir -p build && cd build && cmake .. [flags] && make -j\$(nproc)"
    exit 1
fi

FFI_LIB="${ROOT}/target/release/libwaku_a2a_ffi.so"
if [ ! -f "$FFI_LIB" ]; then
    echo "ERROR: libwaku_a2a_ffi.so not found in target/release/. Build with:"
    echo "  cargo build --release -p waku-a2a-ffi"
    exit 1
fi

# Stage files
STAGING=$(mktemp -d)
trap "rm -rf $STAGING" EXIT

mkdir -p "${STAGING}/${PACKAGE_NAME}/lib"
mkdir -p "${STAGING}/${PACKAGE_NAME}/qml"
mkdir -p "${STAGING}/${PACKAGE_NAME}/icons"

cp "${BUILD_DIR}/libmessaging_a2a_ui.so" "${STAGING}/${PACKAGE_NAME}/lib/"
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
