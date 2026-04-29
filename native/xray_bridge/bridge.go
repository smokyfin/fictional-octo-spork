// Package main exposes a small C ABI over xtls/Xray-core and
// xjasonlyu/tun2socks so the Rust core can run a Xray instance and bridge
// the Android TUN file descriptor into it without going through gomobile.
//
// Build (see ../../.github/workflows/rust.yml and the README in this
// directory):
//
//	GOOS=android GOARCH=arm64 CGO_ENABLED=1 \
//	  CC=$NDK/.../aarch64-linux-android24-clang \
//	  go build -buildmode=c-shared -trimpath \
//	    -o ../../flutter_app/android/app/src/main/jniLibs/arm64-v8a/libxray_bridge.so .
//
// The companion header (libxray_bridge.h) is consumed by the Rust core's
// `xray_runtime` module. All exported functions are safe to call from any
// thread; mutation of the singleton instance is guarded by a Go-side
// sync.Mutex.
package main

/*
#include <stdlib.h>
*/
import "C"

import (
	"fmt"
	"sync"
	"time"
	"unsafe"

	xraycore "github.com/xtls/xray-core/core"
	_ "github.com/xtls/xray-core/main/distro/all"

	"github.com/xjasonlyu/tun2socks/v2/engine"
	t2slog "github.com/xjasonlyu/tun2socks/v2/log"

	"go.uber.org/zap"
	"go.uber.org/zap/zapcore"
)

func init() {
	// xjasonlyu/tun2socks installs a zap logger whose Fatal level calls
	// `os.Exit(1)` (zap default `OnFatal`). When `engine.Start()` fails we
	// don't want to terminate the host process — we need a recoverable
	// panic instead. Replace the global logger with one whose Fatal hook
	// panics; we recover() in `tun2socks_start` below.
	cfg := zap.NewProductionConfig()
	cfg.Level.SetLevel(zap.WarnLevel)
	logger, err := cfg.Build(zap.OnFatal(zapcore.WriteThenPanic))
	if err == nil {
		t2slog.SetLogger(logger)
	}
}

var (
	mu      sync.Mutex
	xrayIns *xraycore.Instance
	tunUp   bool
)

// xray_start parses a JSON configuration in xray-core format and brings up
// a single in-process xray instance. Returns an empty string on success,
// otherwise an error message which the caller MUST free with FreeCString.
//
//export xray_start
func xray_start(configJSON *C.char) *C.char {
	mu.Lock()
	defer mu.Unlock()

	if xrayIns != nil {
		return C.CString("xray instance already running")
	}
	jsonBytes := []byte(C.GoString(configJSON))
	cfg, err := xraycore.LoadConfig("json", jsonBytes)
	if err != nil {
		return C.CString(fmt.Sprintf("load config: %v", err))
	}
	ins, err := xraycore.New(cfg)
	if err != nil {
		return C.CString(fmt.Sprintf("new instance: %v", err))
	}
	if err := ins.Start(); err != nil {
		return C.CString(fmt.Sprintf("start: %v", err))
	}
	xrayIns = ins
	return C.CString("")
}

// xray_stop shuts down the running xray instance, if any. Always returns
// an empty string on success or a textual error otherwise. Safe to call
// when no instance is running (in which case it succeeds silently).
//
//export xray_stop
func xray_stop() *C.char {
	mu.Lock()
	defer mu.Unlock()

	if xrayIns == nil {
		return C.CString("")
	}
	if err := xrayIns.Close(); err != nil {
		xrayIns = nil
		return C.CString(fmt.Sprintf("close: %v", err))
	}
	xrayIns = nil
	return C.CString("")
}

// tun2socks_start wires the Android TUN file descriptor into the
// `xjasonlyu/tun2socks/v2` engine, which terminates IP packets to a SOCKS5
// proxy. fd is duplicated by the engine, mtu is the interface MTU,
// proxyURL is e.g. "socks5://127.0.0.1:1080", and udpTimeoutMs controls
// idle UDP session expiry.
//
//export tun2socks_start
func tun2socks_start(
	fd C.int,
	mtu C.int,
	proxyURL *C.char,
	udpTimeoutMs C.int,
) *C.char {
	mu.Lock()
	defer mu.Unlock()

	if tunUp {
		return C.CString("tun2socks engine already running")
	}
	key := &engine.Key{
		// fd:// scheme is recognised by the gvisor-based stack adapter.
		Device:     fmt.Sprintf("fd://%d", int(fd)),
		MTU:        int(mtu),
		Proxy:      C.GoString(proxyURL),
		LogLevel:   "warning",
		UDPTimeout: time.Duration(int(udpTimeoutMs)) * time.Millisecond,
	}
	engine.Insert(key)

	// engine.Start panics through the (re-configured) zap logger if any
	// of its initialisers fail. Capture that here and surface it as a
	// regular error string so the caller can show it in the UI.
	var startErr error
	func() {
		defer func() {
			if r := recover(); r != nil {
				startErr = fmt.Errorf("%v", r)
			}
		}()
		engine.Start()
	}()
	if startErr != nil {
		return C.CString(fmt.Sprintf("engine start: %v", startErr))
	}
	tunUp = true
	return C.CString("")
}

// tun2socks_stop tears down the running tun2socks engine. The TUN fd is
// closed by the engine; the caller must not use it afterwards.
//
//export tun2socks_stop
func tun2socks_stop() *C.char {
	mu.Lock()
	defer mu.Unlock()

	if !tunUp {
		return C.CString("")
	}
	var stopErr error
	func() {
		defer func() {
			if r := recover(); r != nil {
				stopErr = fmt.Errorf("%v", r)
			}
		}()
		engine.Stop()
	}()
	tunUp = false
	if stopErr != nil {
		return C.CString(fmt.Sprintf("engine stop: %v", stopErr))
	}
	return C.CString("")
}

// FreeCString releases a *C.char allocated by any of the bridge functions.
//
//export FreeCString
func FreeCString(s *C.char) {
	if s != nil {
		C.free(unsafe.Pointer(s))
	}
}

func main() {}
