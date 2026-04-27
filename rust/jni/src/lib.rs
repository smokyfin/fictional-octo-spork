//! Android JNI shim that delegates to `ff_vpn_core::ffi`.
//!
//! The Java/Kotlin side declares these as native methods on
//! `com.incss.ff.vpn.NativeBridge`.

#![cfg(target_os = "android")]

use ff_vpn_core::{config::AppConfig, engine::PlatformContext, pt::Provider};
use jni::objects::{JClass, JString};
use jni::sys::{jint, jlong, jstring};
use jni::JNIEnv;
use std::path::PathBuf;

#[no_mangle]
pub extern "system" fn Java_com_incss_ff_vpn_NativeBridge_init(_env: JNIEnv, _class: JClass) {
    ff_vpn_core::init_logging();
}

#[no_mangle]
pub extern "system" fn Java_com_incss_ff_vpn_NativeBridge_start(
    mut env: JNIEnv,
    _class: JClass,
    config_json: JString,
    private_dir: JString,
    tun_fd: jint,
    tun_addr: JString,
    tun_mtu: jint,
) -> jint {
    let result: anyhow::Result<()> = (|| {
        let cfg: String = env.get_string(&config_json)?.into();
        let dir: String = env.get_string(&private_dir)?.into();
        let addr: String = env.get_string(&tun_addr)?.into();
        let parsed: AppConfig = serde_json::from_str(&cfg)?;
        let ctx = PlatformContext {
            tun_fd,
            tun_addr: addr.parse()?,
            tun_mtu: tun_mtu as u16,
            private_dir: PathBuf::from(dir),
            pt_provider: Provider::Leaf,
        };
        ff_vpn_core::start(parsed, ctx)?;
        Ok(())
    })();
    match result {
        Ok(()) => 0,
        Err(e) => {
            tracing::error!(?e, "JNI start failed");
            1
        }
    }
}

#[no_mangle]
pub extern "system" fn Java_com_incss_ff_vpn_NativeBridge_stop(
    _env: JNIEnv,
    _class: JClass,
) -> jint {
    match ff_vpn_core::stop() {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

#[no_mangle]
pub extern "system" fn Java_com_incss_ff_vpn_NativeBridge_statusJson<'a>(
    env: JNIEnv<'a>,
    _class: JClass<'a>,
) -> jstring {
    let s = serde_json::to_string(&ff_vpn_core::status()).unwrap_or_else(|_| "{}".into());
    env.new_string(s)
        .map(|j| j.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

#[no_mangle]
pub extern "system" fn Java_com_incss_ff_vpn_NativeBridge_ipcAuthToken<'a>(
    env: JNIEnv<'a>,
    _class: JClass<'a>,
) -> jstring {
    let t = ff_vpn_core::ipc::current_auth_token();
    env.new_string(t)
        .map(|j| j.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

// Suppress unused-jlong import warning.
#[allow(dead_code)]
fn _silence() -> jlong {
    0
}
