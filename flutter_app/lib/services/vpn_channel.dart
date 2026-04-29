import 'dart:async';

import 'package:flutter/services.dart';

/// Thin wrapper around the platform-specific VPN bridge.
///
/// Android: implemented in `MainActivity.kt` and `VpnForegroundService.kt`,
/// which start `FfVpnService` (a `VpnService`) and then JNI into the Rust
/// core.
///
/// iOS: implemented in `AppDelegate.swift`, which uses `NetworkExtension`
/// (`NETunnelProviderManager` + `NEPacketTunnelProvider`).
class VpnChannel {
  static const _ch = MethodChannel('ff.vpn/control');
  static const _events = EventChannel('ff.vpn/events');

  Future<void> requestPermission() => _ch.invokeMethod<void>('requestPermission');

  Future<bool> hasPermission() async =>
      (await _ch.invokeMethod<bool>('hasPermission')) ?? false;

  Future<void> connect({
    required String configJson,
    String? exitCountry,
    List<String>? allowedPackages,
    List<String>? disallowedPackages,
    bool routeThroughTor = false,
  }) =>
      _ch.invokeMethod<void>('connect', {
        'config': configJson,
        'exitCountry': exitCountry,
        'allowedPackages': allowedPackages,
        'disallowedPackages': disallowedPackages,
        'routeThroughTor': routeThroughTor,
      });

  Future<void> disconnect() => _ch.invokeMethod<void>('disconnect');

  Future<Map<String, dynamic>> status() async {
    final res = await _ch.invokeMethod<Map>('status');
    return res?.cast<String, dynamic>() ?? <String, dynamic>{};
  }

  Stream<Map<String, dynamic>> events() => _events
      .receiveBroadcastStream()
      .map((e) => Map<String, dynamic>.from(e as Map));
}
