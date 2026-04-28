//! Android JNI shim that delegates to `ff_vpn_core::ffi`.
//!
//! The Java/Kotlin side declares these as native methods on
//! `com.incss.ff.vpn.NativeBridge`.
//!
//! `jni 0.22` split `JNIEnv` into `EnvUnowned` (FFI-safe, what the JVM
//! hands us) and `Env` (the full-API handle obtained via
//! `EnvUnowned::with_env`). All native methods take `EnvUnowned<'local>`
//! and lift it into an `Env` for the duration of the body. The
//! `with_env` body always returns `Ok(_)` — we map every internal
//! failure to a return code so the JVM-facing `ErrorPolicy`
//! (`ThrowRuntimeExAndDefault`) only fires on a Rust panic.

#![cfg(target_os = "android")]

use ff_vpn_core::{config::AppConfig, engine::PlatformContext, pt::Provider, util::FdGuard};
use jni::errors::{Result as JniResult, ThrowRuntimeExAndDefault};
use jni::objects::{JClass, JString};
use jni::sys::{jint, jstring};
use jni::EnvUnowned;
use std::path::PathBuf;

#[no_mangle]
pub extern "system" fn Java_com_incss_ff_vpn_NativeBridge_init<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
) {
    env.with_env(|_env| -> JniResult<()> {
        ff_vpn_core::init_logging();
        Ok(())
    })
    .resolve::<ThrowRuntimeExAndDefault>();
}

#[no_mangle]
pub extern "system" fn Java_com_incss_ff_vpn_NativeBridge_start<'local>(
    mut unowned_env: EnvUnowned<'local>,
    _class: JClass<'local>,
    config_json: JString<'local>,
    private_dir: JString<'local>,
    tun_fd: jint,
    tun_addr: JString<'local>,
    tun_mtu: jint,
) -> jint {
    // Arm a fd-close guard the moment we have the raw fd. Any failure in
    // the parse/conversion steps below closes the fd. The guard is
    // disarmed once `ff_vpn_core::start` takes over — that function arms
    // its own internal guard, so ownership is continuously held by
    // exactly one party until either bring-up succeeds or the fd is
    // closed.
    let fd_guard = FdGuard::new(tun_fd);

    unowned_env
        .with_env(|env| -> JniResult<jint> {
            let inner = || -> anyhow::Result<()> {
                let cfg: String = config_json
                    .to_string(env)
                    .map_err(|e| anyhow::anyhow!("to_string config: {e}"))?;
                let dir: String = private_dir
                    .to_string(env)
                    .map_err(|e| anyhow::anyhow!("to_string private_dir: {e}"))?;
                let addr: String = tun_addr
                    .to_string(env)
                    .map_err(|e| anyhow::anyhow!("to_string tun_addr: {e}"))?;
                let parsed: AppConfig = serde_json::from_str(&cfg)?;
                let ctx = PlatformContext {
                    tun_fd,
                    tun_addr: addr.parse()?,
                    tun_mtu: tun_mtu as u16,
                    private_dir: PathBuf::from(dir),
                    pt_provider: Provider::Leaf,
                };
                // Hand the fd to the core; it owns close-on-failure from here.
                fd_guard.disarm();
                ff_vpn_core::start(parsed, ctx)?;
                Ok(())
            };
            Ok(match inner() {
                Ok(()) => 0,
                Err(e) => {
                    tracing::error!(?e, "JNI start failed");
                    1
                }
            })
        })
        .resolve::<ThrowRuntimeExAndDefault>()
}

#[no_mangle]
pub extern "system" fn Java_com_incss_ff_vpn_NativeBridge_stop<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
) -> jint {
    env.with_env(|_env| -> JniResult<jint> {
        Ok(match ff_vpn_core::stop() {
            Ok(()) => 0,
            Err(_) => 1,
        })
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

#[no_mangle]
pub extern "system" fn Java_com_incss_ff_vpn_NativeBridge_statusJson<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
) -> jstring {
    env.with_env(|env| -> JniResult<jstring> {
        let s = serde_json::to_string(&ff_vpn_core::status()).unwrap_or_else(|_| "{}".into());
        Ok(env
            .new_string(&s)
            .map(|j| j.into_raw())
            .unwrap_or(std::ptr::null_mut()))
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

#[no_mangle]
pub extern "system" fn Java_com_incss_ff_vpn_NativeBridge_ipcAuthToken<'local>(
    mut env: EnvUnowned<'local>,
    _class: JClass<'local>,
) -> jstring {
    env.with_env(|env| -> JniResult<jstring> {
        let t = ff_vpn_core::ipc::current_auth_token();
        Ok(env
            .new_string(&t)
            .map(|j| j.into_raw())
            .unwrap_or(std::ptr::null_mut()))
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}
