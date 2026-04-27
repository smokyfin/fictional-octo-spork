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
import android.util.Log
import org.json.JSONObject

/**
 * Foreground `VpnService` that owns the TUN file descriptor and hands it to
 * the Rust core. All actual VPN logic (Leaf, Arti, DNS, IPC) lives in Rust.
 */
class FfVpnService : VpnService() {

    private var tun: ParcelFileDescriptor? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_START -> startTunnel(intent)
            ACTION_STOP -> stopTunnel()
        }
        return START_NOT_STICKY
    }

    private fun startTunnel(intent: Intent) {
        ensureForeground()
        val configJson = intent.getStringExtra(EXTRA_CONFIG) ?: return stopSelfSafely()
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

        val pfd = builder.establish() ?: return stopSelfSafely()
        tun = pfd

        val privateDir = filesDir.absolutePath
        try {
            // Hand the duped FD to Rust, which keeps ownership.
            val fd = pfd.detachFd()
            NativeBridge.init()
            val parsed = JSONObject(configJson) // sanity check
            val rc = NativeBridge.start(parsed.toString(), privateDir, fd, tunAddr, mtu)
            if (rc != 0) {
                Log.e(TAG, "NativeBridge.start returned $rc")
                stopTunnel()
            } else {
                VpnEventBus.emit(mapOf("kind" to "status", "running" to true))
            }
        } catch (t: Throwable) {
            Log.e(TAG, "startTunnel failed", t)
            VpnEventBus.emit(mapOf("kind" to "error", "message" to (t.message ?: "start failed")))
            stopTunnel()
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
            .setSmallIcon(android.R.drawable.stat_sys_vpn_ic)
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
