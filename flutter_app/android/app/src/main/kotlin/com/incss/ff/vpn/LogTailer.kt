package com.incss.ff.vpn

import android.os.Process as AndroidProcess
import android.util.Log
import java.io.BufferedReader
import java.io.InputStreamReader
import java.lang.Process as JavaProcess
import kotlin.concurrent.thread

/**
 * Tails this process's logcat output and forwards each line to
 * [VpnEventBus] as a `kind: "log"` event so the Flutter Developer tab
 * can display the live connection log.
 *
 * `--pid=<own pid>` is allowed for unprivileged apps since Android 7+, so
 * we don't need the privileged `READ_LOGS` permission.
 */
object LogTailer {
    private var thread: Thread? = null
    private var process: JavaProcess? = null

    @Synchronized
    fun start() {
        if (thread != null) return
        val pid = AndroidProcess.myPid()
        process = try {
            ProcessBuilder("logcat", "-v", "threadtime", "--pid=$pid")
                .redirectErrorStream(true)
                .start()
        } catch (t: Throwable) {
            Log.w("LogTailer", "failed to spawn logcat", t)
            null
        }
        val proc = process ?: return
        thread = thread(name = "ff-vpn-logcat", isDaemon = true) {
            try {
                BufferedReader(InputStreamReader(proc.inputStream)).useLines { lines ->
                    for (line in lines) {
                        // Skip the noise from VpnEventBus / Flutter platform
                        // channels so we don't feed our own log messages
                        // back into the bus.
                        if (line.contains("flutter") ||
                            line.contains("FlutterJNI") ||
                            line.contains("VpnEventBus") ||
                            line.contains("Choreographer")
                        ) continue
                        VpnEventBus.emitLog(line)
                    }
                }
            } catch (_: InterruptedException) {
                // shutdown
            } catch (t: Throwable) {
                Log.w("LogTailer", "logcat reader died", t)
            }
        }
    }

    @Synchronized
    fun stop() {
        try {
            process?.destroy()
        } catch (_: Throwable) {
        }
        process = null
        thread?.interrupt()
        thread = null
    }
}
