package com.incss.ff.vpn

import android.app.Activity
import android.content.Intent
import android.net.VpnService
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.EventChannel
import io.flutter.plugin.common.MethodChannel

class MainActivity : FlutterActivity() {
    private val controlChannelName = "ff.vpn/control"
    private val eventsChannelName = "ff.vpn/events"
    private var pendingResult: MethodChannel.Result? = null
    private var pendingArgs: Map<String, Any?>? = null

    private val prepareCode = 0xFEED

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        val control = MethodChannel(flutterEngine.dartExecutor.binaryMessenger, controlChannelName)
        control.setMethodCallHandler { call, result ->
            when (call.method) {
                "hasPermission" -> {
                    val intent = VpnService.prepare(this)
                    result.success(intent == null)
                }
                "requestPermission" -> {
                    val intent = VpnService.prepare(this)
                    if (intent == null) {
                        result.success(true)
                    } else {
                        pendingResult = result
                        pendingArgs = null
                        startActivityForResult(intent, prepareCode)
                    }
                }
                "connect" -> {
                    val args = call.arguments as Map<*, *>
                    @Suppress("UNCHECKED_CAST")
                    val typed = args as Map<String, Any?>
                    val intent = VpnService.prepare(this)
                    if (intent != null) {
                        pendingResult = result
                        pendingArgs = typed
                        startActivityForResult(intent, prepareCode)
                    } else {
                        startVpn(typed)
                        result.success(null)
                    }
                }
                "disconnect" -> {
                    val intent = Intent(this, FfVpnService::class.java).apply {
                        action = FfVpnService.ACTION_STOP
                    }
                    startService(intent)
                    result.success(null)
                }
                "status" -> {
                    val json = NativeBridge.statusJson()
                    @Suppress("UNCHECKED_CAST")
                    result.success(parseJson(json))
                }
                else -> result.notImplemented()
            }
        }

        EventChannel(flutterEngine.dartExecutor.binaryMessenger, eventsChannelName)
            .setStreamHandler(VpnEventBus)
    }

    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == prepareCode) {
            val ok = resultCode == Activity.RESULT_OK
            val args = pendingArgs
            val pending = pendingResult
            pendingResult = null
            pendingArgs = null
            if (!ok) {
                pending?.error("PERMISSION_DENIED", "VPN permission was not granted", null)
                return
            }
            if (args != null) {
                startVpn(args)
                pending?.success(null)
            } else {
                pending?.success(true)
            }
        }
    }

    private fun startVpn(args: Map<String, Any?>) {
        val configJson = args["config"] as String
        val exitCountry = args["exitCountry"] as String?
        @Suppress("UNCHECKED_CAST")
        val allowed = args["allowedPackages"] as List<String>?
        @Suppress("UNCHECKED_CAST")
        val disallowed = args["disallowedPackages"] as List<String>?

        val intent = Intent(this, FfVpnService::class.java).apply {
            action = FfVpnService.ACTION_START
            putExtra(FfVpnService.EXTRA_CONFIG, configJson)
            putExtra(FfVpnService.EXTRA_EXIT_COUNTRY, exitCountry)
            putStringArrayListExtra(
                FfVpnService.EXTRA_ALLOWED, allowed?.let(::ArrayList)
            )
            putStringArrayListExtra(
                FfVpnService.EXTRA_DISALLOWED, disallowed?.let(::ArrayList)
            )
        }
        startForegroundService(intent)
    }

    private fun parseJson(json: String): Map<String, Any?> {
        return try {
            val obj = org.json.JSONObject(json)
            val m = mutableMapOf<String, Any?>()
            obj.keys().forEach { k -> m[k] = obj.opt(k) }
            m
        } catch (_: Throwable) {
            emptyMap()
        }
    }
}
