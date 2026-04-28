package com.incss.ff.vpn

import android.os.Handler
import android.os.Looper
import io.flutter.plugin.common.EventChannel

object VpnEventBus : EventChannel.StreamHandler {
    private var sink: EventChannel.EventSink? = null
    private val main = Handler(Looper.getMainLooper())

    override fun onListen(arguments: Any?, events: EventChannel.EventSink?) {
        sink = events
    }

    override fun onCancel(arguments: Any?) {
        sink = null
    }

    fun emit(payload: Map<String, Any?>) {
        main.post { sink?.success(payload) }
    }

    fun emitLog(line: String) {
        emit(mapOf("kind" to "log", "line" to line))
    }
}
