#!/usr/bin/env bash
# Build the Rust JNI cdylib for all Android ABIs and place the .so files
# where Gradle's `jniLibs` source-set picks them up.
#
# Requires:
#   - Android NDK (set ANDROID_NDK_HOME)
#   - cargo-ndk (`cargo install cargo-ndk`)
#   - Rust nightly stable (>= 1.83) with android targets:
#       rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android

set -euo pipefail

cd "$(dirname "$0")/../rust"

ABIS=(arm64-v8a armeabi-v7a x86_64)

cargo ndk \
    -t arm64-v8a \
    -t armeabi-v7a \
    -t x86_64 \
    -o ../flutter_app/android/app/src/main/jniLibs \
    -- build --release -p ff_vpn_jni
