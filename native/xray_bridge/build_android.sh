#!/usr/bin/env bash
# Cross-compile native/xray_bridge into the jniLibs directory consumed by
# both `rust/core/build.rs` (linker -L) and the Android packager (APK lib/).
#
# Usage:
#   ./build_android.sh                   # builds arm64-v8a (default)
#   ABI=armeabi-v7a ./build_android.sh   # cross-compile for 32-bit ARM
#
# Required environment:
#   GO_BIN              path to a Go toolchain >= 1.22 (default: `go`)
#   ANDROID_NDK_HOME    path to an Android NDK r26+ install
set -euo pipefail

ABI="${ABI:-arm64-v8a}"
GO_BIN="${GO_BIN:-go}"
NDK="${ANDROID_NDK_HOME:?ANDROID_NDK_HOME must be set}"

case "${ABI}" in
  arm64-v8a)     GOARCH=arm64;   CLANG="aarch64-linux-android24-clang"   ;;
  armeabi-v7a)   GOARCH=arm;     CLANG="armv7a-linux-androideabi24-clang"; export GOARM=7 ;;
  x86_64)        GOARCH=amd64;   CLANG="x86_64-linux-android24-clang"    ;;
  x86)           GOARCH=386;     CLANG="i686-linux-android24-clang"      ;;
  *) echo "unsupported ABI: ${ABI}" >&2; exit 64 ;;
esac

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUT_DIR="${SCRIPT_DIR}/../../flutter_app/android/app/src/main/jniLibs/${ABI}"
mkdir -p "${OUT_DIR}"

CC="${NDK}/toolchains/llvm/prebuilt/linux-x86_64/bin/${CLANG}"

echo "Building libxray_bridge.so for ${ABI} (GOARCH=${GOARCH}) -> ${OUT_DIR}"
cd "${SCRIPT_DIR}"
CGO_ENABLED=1 GOOS=android GOARCH="${GOARCH}" CC="${CC}" \
  "${GO_BIN}" build \
    -buildmode=c-shared \
    -trimpath \
    -ldflags="-s -w" \
    -o "${OUT_DIR}/libxray_bridge.so" \
    .
ls -lh "${OUT_DIR}/libxray_bridge.so"
