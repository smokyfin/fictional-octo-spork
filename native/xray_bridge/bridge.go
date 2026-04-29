// Package main exposes a small C ABI over xtls/Xray-core so the Rust
// core can run a Xray instance without going through gomobile.
//
// The TUN ↔ SOCKS5 bridge is handled by hev-socks5-tunnel (a separate
// C library, see ../hev_socks5_tunnel/) — that is no longer this
// package's responsibility.
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
	"unsafe"

	xraycore "github.com/xtls/xray-core/core"
	_ "github.com/xtls/xray-core/main/distro/all"
)

var (
	mu      sync.Mutex
	xrayIns *xraycore.Instance
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

// FreeCString releases a *C.char allocated by any of the bridge functions.
//
//export FreeCString
func FreeCString(s *C.char) {
	if s != nil {
		C.free(unsafe.Pointer(s))
	}
}

func main() {}
