#!/usr/bin/env bash
# Cross-compile hev-socks5-tunnel into the jniLibs directory of the
# Android Flutter app. Idempotent — clones the upstream repo if missing,
# then runs `ndk-build`.
#
# Usage:
#   ./build_android.sh                   # arm64-v8a (default)
#   ABI=armeabi-v7a ./build_android.sh
#   ABI=x86_64 ./build_android.sh
#
# Requires: ANDROID_NDK_HOME (or ANDROID_NDK_ROOT) pointing at an NDK that
# includes ndk-build.
set -euo pipefail

ABI="${ABI:-arm64-v8a}"

NDK="${ANDROID_NDK_HOME:-${ANDROID_NDK_ROOT:-}}"
if [[ -z "${NDK}" ]]; then
    echo "ERROR: set ANDROID_NDK_HOME or ANDROID_NDK_ROOT" >&2
    exit 1
fi

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${HERE}/../.." && pwd)"
JNILIBS_DIR="${WORKSPACE_ROOT}/flutter_app/android/app/src/main/jniLibs/${ABI}"

# ndk-build expects the source tree under jni/. We clone the upstream
# repository (with submodules — yaml, lwip, hev-task-system) into
# `<HERE>/jni` if it isn't already there.
SRC_DIR="${HERE}/jni"
HEV_REV="${HEV_REV:-}"     # optional: pinned commit
HEV_REPO="https://github.com/heiher/hev-socks5-tunnel.git"

if [[ ! -d "${SRC_DIR}/.git" ]]; then
    echo "Cloning ${HEV_REPO} into ${SRC_DIR}..."
    rm -rf "${SRC_DIR}"
    git clone --depth 1 --recurse-submodules "${HEV_REPO}" "${SRC_DIR}"
    if [[ -n "${HEV_REV}" ]]; then
        (cd "${SRC_DIR}" && git fetch --depth 1 origin "${HEV_REV}" && \
            git checkout "${HEV_REV}" && \
            git submodule update --init --recursive)
    fi
fi

mkdir -p "${JNILIBS_DIR}"

echo "Building libhev-socks5-tunnel.so (${ABI}) with ndk-build..."
"${NDK}/ndk-build" -C "${HERE}" \
    APP_ABI="${ABI}" \
    NDK_PROJECT_PATH="${HERE}" \
    APP_BUILD_SCRIPT="${SRC_DIR}/Android.mk" \
    APP_PLATFORM="android-29" \
    NDK_LIBS_OUT="${HERE}/libs" \
    NDK_OUT="${HERE}/obj"

cp "${HERE}/libs/${ABI}/libhev-socks5-tunnel.so" "${JNILIBS_DIR}/"
echo "Installed: ${JNILIBS_DIR}/libhev-socks5-tunnel.so"
