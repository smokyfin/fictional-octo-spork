# `xray_bridge` — Go cgo shim for xray-core + tun2socks

This package wraps two upstream Go libraries —
[`xtls/xray-core`](https://github.com/XTLS/Xray-core) and
[`xjasonlyu/tun2socks/v2`](https://github.com/xjasonlyu/tun2socks) — behind
a tiny C ABI that the Rust core (`rust/core/src/engine/xray_runtime.rs`)
calls via FFI. We use it instead of `gomobile`/AAR so the same artefact
can serve both the Android JNI library and a future iOS Network Extension
build.

## Exported symbols

| Symbol            | Signature                                                                   | Notes                                                                  |
|-------------------|-----------------------------------------------------------------------------|------------------------------------------------------------------------|
| `xray_start`      | `char* xray_start(char* configJSON)`                                        | Returns `""` on success, or an error string. Caller must `FreeCString`. |
| `xray_stop`       | `char* xray_stop()`                                                          | Idempotent.                                                            |
| `tun2socks_start` | `char* tun2socks_start(int fd, int mtu, char* proxyURL, int udpTimeoutMs)` | `proxyURL` like `socks5://127.0.0.1:10808`.                            |
| `tun2socks_stop`  | `char* tun2socks_stop()`                                                     | Closes the duplicated TUN fd internally.                               |
| `FreeCString`     | `void FreeCString(char* s)`                                                  | Releases any C string returned by the bridge.                          |

## Build

```
export PATH=/usr/local/go/bin:$PATH
export ANDROID_NDK_HOME=$HOME/android-sdk/ndk/26.3.11579264
export CC=$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android24-clang
cd native/xray_bridge
CGO_ENABLED=1 GOOS=android GOARCH=arm64 \
  go build -buildmode=c-shared -trimpath \
    -o ../../flutter_app/android/app/src/main/jniLibs/arm64-v8a/libxray_bridge.so \
    .
```

The output is consumed at link time by `rust/core/build.rs` (under
`cfg(target_os = "android")`) and at runtime by `System.loadLibrary` in the
Kotlin VPN service.

## Why c-shared and not c-archive?

Go's `c-archive` build mode is unsupported on `android/arm64` (the Go
toolchain explicitly rejects it). `c-shared` produces a regular ELF
shared object the dynamic linker is happy to load alongside our
`libff_vpn_jni.so`.

## Why we override the tun2socks logger

`xjasonlyu/tun2socks` calls `log.Fatalf` (zap → `os.Exit(1)`) when its
internal `engine.Start()` fails. That is fatal for an embedded use-case;
we override the global zap logger with `OnFatal: WriteThenPanic` so the
panic can be `recover`ed from `tun2socks_start` and surfaced as a normal
error string. See `bridge.go::init`.
