package com.incss.ff.vpn

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.net.VpnService
import android.os.Build
import android.os.ParcelFileDescriptor
import android.system.ErrnoException
import android.system.Os
import android.util.Log
import org.json.JSONObject

/**
 * Foreground `VpnService` that owns the TUN file descriptor and hands it to
 * the Rust core. All actual VPN logic (Leaf, Arti, DNS, IPC) lives in Rust.
 */
class FfVpnService : VpnService() {

    private var tun: ParcelFileDescriptor? = null

    override fun onCreate() {
        super.onCreate()
        LogTailer.start()
        VpnEventBus.emitLog("[svc] FfVpnService.onCreate")
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_START -> {
                VpnEventBus.emitLog("[svc] start command received")
                startTunnel(intent)
            }
            ACTION_STOP -> {
                VpnEventBus.emitLog("[svc] stop command received")
                stopTunnel()
            }
        }
        return START_NOT_STICKY
    }

    private fun startTunnel(intent: Intent) {
        ensureForeground()
        val configJson = intent.getStringExtra(EXTRA_CONFIG) ?: return stopSelfSafely()
        val exitCountry = intent.getStringExtra(EXTRA_EXIT_COUNTRY)
        val allowed = intent.getStringArrayListExtra(EXTRA_ALLOWED)
        val disallowed = intent.getStringArrayListExtra(EXTRA_DISALLOWED)

        val tunAddr = "10.10.0.2"
        val mtu = 1500

        val builder = Builder()
            .setSession("ff-vpn")
            .setMtu(mtu)
            .addAddress(tunAddr, 24)
            .addRoute("0.0.0.0", 0)
            .addRoute("::", 0)
            // Our embedded DNS server lives at the TUN address.
            .addDnsServer(tunAddr)
            .setBlocking(true)

        // Per-app routing: allowlist takes precedence; if neither set, route
        // everything through the VPN.
        if (!allowed.isNullOrEmpty()) {
            for (p in allowed) {
                runCatching { builder.addAllowedApplication(p) }
            }
        } else if (!disallowed.isNullOrEmpty()) {
            for (p in disallowed) {
                runCatching { builder.addDisallowedApplication(p) }
            }
        }

        VpnEventBus.emitLog("[svc] establishing TUN: addr=$tunAddr/24 mtu=$mtu")
        val pfd = builder.establish() ?: run {
            VpnEventBus.emitLog("[svc] TUN.establish() returned null (permission denied?)")
            return stopSelfSafely()
        }
        tun = pfd

        val privateDir = filesDir.absolutePath
        // Detach so the Rust core can own the FD. fd-ownership contract:
        //   - Once `NativeBridge.start` is invoked, the Rust core owns the
        //     fd and is responsible for closing it on any failure (via its
        //     `FdGuard`). We MUST NOT close it here on rc != 0 — doing so
        //     races with Rust and could destroy an unrelated fd that POSIX
        //     reassigned in the meantime.
        //   - If we throw BEFORE `NativeBridge.start` is called (JSON parse,
        //     `exitCountry` injection, JNI marshalling), Rust never saw the
        //     fd and we MUST close it here. The `nativeStartCalled` flag
        //     distinguishes the two cases.
        val fd = pfd.detachFd()
        VpnEventBus.emitLog("[svc] TUN fd detached: $fd, private dir: $privateDir")
        var nativeStartCalled = false
        try {
            NativeBridge.init()
            VpnEventBus.emitLog("[svc] NativeBridge.init() ok")
            val parsed = JSONObject(configJson)
            // Inject the user-selected Tor exit country into the config blob
            // under `user.exit_country` — the Rust `AppConfig::user::exit_country`
            // field is what `arti_runtime.rs` reads when applying StreamPrefs.
            // Without this the country picker would silently no-op.
            if (!exitCountry.isNullOrBlank()) {
                val user = parsed.optJSONObject("user") ?: JSONObject()
                user.put("exit_country", exitCountry)
                parsed.put("user", user)
            }
            nativeStartCalled = true
            VpnEventBus.emitLog("[svc] calling NativeBridge.start (exitCountry=${exitCountry ?: "-"})")
            val rc = NativeBridge.start(parsed.toString(), privateDir, fd, tunAddr, mtu)
            if (rc != 0) {
                val err = runCatching { NativeBridge.lastError() }.getOrNull()
                Log.e(TAG, "NativeBridge.start returned $rc: ${err ?: "(no error string)"}")
                VpnEventBus.emitLog("[svc] NativeBridge.start rc=$rc: ${err ?: "(no error)"}")
                if (err != null) {
                    VpnEventBus.emit(mapOf("kind" to "error", "message" to err))
                }
                stopTunnel()
            } else {
                VpnEventBus.emitLog("[svc] NativeBridge.start ok; tunnel up")
                VpnEventBus.emit(mapOf("kind" to "status", "running" to true))
            }
        } catch (t: Throwable) {
            Log.e(TAG, "startTunnel failed", t)
            VpnEventBus.emitLog("[svc] startTunnel failed: ${t.javaClass.simpleName}: ${t.message}")
            VpnEventBus.emit(mapOf("kind" to "error", "message" to (t.message ?: "start failed")))
            if (!nativeStartCalled) closeRawFd(fd)
            stopTunnel()
        }
    }

    private fun closeRawFd(fd: Int) {
        try {
            // Wrap-and-close is the public API for closing a raw fd that
            // originated from ParcelFileDescriptor.detachFd().
            ParcelFileDescriptor.adoptFd(fd).close()
        } catch (t: Throwable) {
            // Last-ditch fallback via libcore if adoptFd somehow fails.
            try {
                Os.close(java.io.FileDescriptor().also { f ->
                    val field = java.io.FileDescriptor::class.java.getDeclaredField("descriptor")
                    field.isAccessible = true
                    field.setInt(f, fd)
                })
            } catch (_: ErrnoException) {
                // ignore
            } catch (_: Throwable) {
                // ignore — best effort cleanup
            }
            Log.w(TAG, "closeRawFd($fd) failed via adoptFd", t)
        }
    }

    private fun stopTunnel() {
        try {
            NativeBridge.stop()
        } catch (t: Throwable) {
            Log.w(TAG, "NativeBridge.stop threw", t)
        }
        tun?.runCatching { close() }
        tun = null
        VpnEventBus.emit(mapOf("kind" to "status", "running" to false))
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    private fun stopSelfSafely() {
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    private fun ensureForeground() {
        val nm = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val ch = NotificationChannel(
                NOTIF_CHANNEL_ID,
                "VPN",
                NotificationManager.IMPORTANCE_LOW
            )
            nm.createNotificationChannel(ch)
        }
        val pi = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE
        )
        val notif: Notification = Notification.Builder(this, NOTIF_CHANNEL_ID)
            .setContentTitle("ff-vpn")
            .setContentText("Routing through Tor + VLESS")
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentIntent(pi)
            .setOngoing(true)
            .build()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            startForeground(
                NOTIF_ID,
                notif,
                ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE
            )
        } else {
            startForeground(NOTIF_ID, notif)
        }
    }

    override fun onDestroy() {
        stopTunnel()
        VpnEventBus.emitLog("[svc] FfVpnService.onDestroy")
        LogTailer.stop()
        super.onDestroy()
    }

    companion object {
        const val ACTION_START = "com.incss.ff.vpn.START"
        const val ACTION_STOP = "com.incss.ff.vpn.STOP"
        const val EXTRA_CONFIG = "config"
        const val EXTRA_EXIT_COUNTRY = "exit_country"
        const val EXTRA_ALLOWED = "allowed"
        const val EXTRA_DISALLOWED = "disallowed"
        private const val NOTIF_ID = 0xC0DE
        private const val NOTIF_CHANNEL_ID = "ff_vpn"
        private const val TAG = "FfVpnService"
    }
}
