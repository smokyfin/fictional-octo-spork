package com.incss.ff.vpn

/**
 * Bindings to the Rust `ff_vpn_jni` cdylib. The signatures must match
 * the `Java_com_incss_ff_vpn_NativeBridge_*` functions in `rust/jni`.
 */
object NativeBridge {
    init {
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
