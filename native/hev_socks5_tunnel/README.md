# hev-socks5-tunnel (vendored)

This directory builds a shared library (`libhev-socks5-tunnel.so`) from
[heiher/hev-socks5-tunnel](https://github.com/heiher/hev-socks5-tunnel),
a small TUN-to-SOCKS5 user-space tunnel written in C.

The library is consumed by the Rust core (`engine::hev_runtime`) to
move IP packets from the OS-owned TUN file descriptor into a local
SOCKS5 listener (Arti when Tor mode is on, xray-core's SOCKS inbound
when `skip_arti=true`).

## Build

```
ANDROID_NDK_HOME=/path/to/ndk ./build_android.sh
ABI=armeabi-v7a ./build_android.sh
ABI=x86_64 ./build_android.sh
```

The script clones the upstream repo (with submodules) into `jni/`
on first run, then invokes `ndk-build` and copies the resulting `.so`
into `flutter_app/android/app/src/main/jniLibs/<abi>/`.

## Exported C ABI (used by Rust)

```
int  hev_socks5_tunnel_main_from_str(const unsigned char* yaml,
                                     unsigned int yaml_len,
                                     int tun_fd);
void hev_socks5_tunnel_quit(void);
void hev_socks5_tunnel_stats(size_t* tx_p, size_t* tx_b,
                             size_t* rx_p, size_t* rx_b);
```

`hev_socks5_tunnel_main_from_str` is **blocking** — it owns the calling
thread until `hev_socks5_tunnel_quit` is invoked from another thread.
The Rust wrapper spawns a dedicated `std::thread` for it.

## Why ndk-build, not CMake

Upstream ships a vetted `Android.mk` + `Application.mk` that wires up
all three submodules (`yaml`, `lwip`, `hev-task-system`). Re-implementing
that as CMake would be a maintenance burden every time hev refactors its
sources. `ndk-build` is shipped with the NDK, so the only build-time
prerequisite is `ANDROID_NDK_HOME`.
