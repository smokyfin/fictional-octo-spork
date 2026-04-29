//! Build script for `ff_vpn_jni`.
//!
//! On Android we link against the prebuilt `libxray_bridge.so` produced by
//! `native/xray_bridge/build_android.sh`. The shared object is consumed by
//! the linker (-L + -lxray_bridge) and lives next to `libff_vpn_jni.so`
//! inside the APK so the dynamic loader can resolve the dependency at
//! runtime.
//!
//! For other targets (host CI, iOS, macOS) we don't link xray at all —
//! `xray_runtime` falls back to a stub that returns
//! `Error::XrayUnavailable`.

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

    let lib_file = lib_dir.join("libxray_bridge.so");
    if !lib_file.exists() {
        // `cargo check` and `cargo clippy` don't link, so the missing .so
        // is fine in CI — emit a warning instead of failing. Real APK
        // builds (`cargo ndk build` → flutter build) will fail at link
        // time if the file is genuinely missing, with a clear error
        // pointing at this build script.
        println!(
            "cargo:warning=libxray_bridge.so not found at {}; \
             run native/xray_bridge/build_android.sh (ABI={abi}) \
             before linking.",
            lib_file.display()
        );
    }

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=dylib=xray_bridge");
    // Ensure the link record uses an unversioned SONAME-less path; the
    // Android dynamic loader matches on file name only.
    println!("cargo:rerun-if-changed={}", lib_file.display());
}
