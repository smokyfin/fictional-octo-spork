package com.incss.ff.vpn

/**
 * Bindings to the Rust `ff_vpn_jni` cdylib. The signatures must match
 * the `Java_com_incss_ff_vpn_NativeBridge_*` functions in `rust/jni`.
 */
object NativeBridge {
    init {
        // Load the Go-side xray-core + tun2socks bridge first so its
        // symbols (xray_start, xray_stop, tun2socks_start, ...) are
        // already resolved by the time we link in libff_vpn_jni, which
        // declares them as NEEDED dynamic dependencies.
        System.loadLibrary("xray_bridge")
        System.loadLibrary("ff_vpn_jni")
    }

    @JvmStatic external fun init()
    @JvmStatic external fun start(
        configJson: String,
        privateDir: String,
        tunFd: Int,
        tunAddr: String,
        tunMtu: Int
    ): Int
    @JvmStatic external fun stop(): Int
    @JvmStatic external fun statusJson(): String
    @JvmStatic external fun ipcAuthToken(): String
    /** Returns the last error from the Rust core, or null if none. Clears it. */
    @JvmStatic external fun lastError(): String?
}
