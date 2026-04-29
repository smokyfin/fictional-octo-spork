//! Build script for `ff_vpn_jni`.
//!
//! On Android we link against:
//!   * `libxray_bridge.so` — built by `native/xray_bridge/build_android.sh`
//!     (Go cgo c-shared blob containing xray-core).
//!   * `libhev-socks5-tunnel.so` — built by
//!     `native/hev_socks5_tunnel/build_android.sh` (C library).
//!
//! Both .so files live next to `libff_vpn_jni.so` inside the APK so the
//! dynamic loader can resolve them at runtime.
//!
//! For other targets (host CI, iOS, macOS) we don't link xray/hev at
//! all — `xray_runtime`/`hev_runtime` fall back to stubs that return
//! `Error::XrayUnavailable` / `Error::HevUnavailable`.

use std::path::PathBuf;

fn main() {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    if target_os != "android" {
        return;
    }
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let abi = match target_arch.as_str() {
        "aarch64" => "arm64-v8a",
        "arm" => "armeabi-v7a",
        "x86_64" => "x86_64",
        "x86" => "x86",
        other => panic!("unsupported android target_arch: {other}"),
    };

    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR is set by cargo"),
    );
    // Workspace root is two levels above rust/jni.
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root resolved from CARGO_MANIFEST_DIR");

    let lib_dir = workspace_root
        .join("flutter_app/android/app/src/main/jniLibs")
        .join(abi);

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rerun-if-changed={}", lib_dir.display());

    // Link xray Go bridge.
    let xray_lib = lib_dir.join("libxray_bridge.so");
    if !xray_lib.exists() {
        println!(
            "cargo:warning=libxray_bridge.so not found at {}; \
             run native/xray_bridge/build_android.sh (ABI={abi}) \
             before linking.",
            xray_lib.display()
        );
    }
    println!("cargo:rustc-link-lib=dylib=xray_bridge");
    println!("cargo:rerun-if-changed={}", xray_lib.display());

    // Link hev-socks5-tunnel C library. The linker name (`-l<name>`)
    // omits the leading `lib` and the `.so` suffix, so for a file
    // named `libhev-socks5-tunnel.so` we pass `hev-socks5-tunnel`.
    let hev_lib = lib_dir.join("libhev-socks5-tunnel.so");
    if !hev_lib.exists() {
        println!(
            "cargo:warning=libhev-socks5-tunnel.so not found at {}; \
             run native/hev_socks5_tunnel/build_android.sh (ABI={abi}) \
             before linking.",
            hev_lib.display()
        );
    }
    println!("cargo:rustc-link-lib=dylib=hev-socks5-tunnel");
    println!("cargo:rerun-if-changed={}", hev_lib.display());
}
