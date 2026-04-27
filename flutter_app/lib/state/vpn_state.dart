import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../services/config_service.dart';
import '../services/vpn_channel.dart';

enum VpnStatus { disconnected, connecting, connected, disconnecting, error }

/// Sentinel used so `copyWith` can distinguish "argument not provided" from
/// "argument explicitly set to null" — without the sentinel, `null` is
/// indistinguishable from a missing parameter and nullable fields can't be
/// cleared.
const Object _unset = Object();

@immutable
class VpnState {
  const VpnState({
    this.status = VpnStatus.disconnected,
    this.config,
    this.exitCountry,
    this.allowedPackages,
    this.disallowedPackages,
    this.errorMessage,
    this.lastStatusJson,
    this.logs = const [],
  });

  final VpnStatus status;
  final AppConfig? config;
  final String? exitCountry;
  final List<String>? allowedPackages;
  final List<String>? disallowedPackages;
  final String? errorMessage;
  final Map<String, dynamic>? lastStatusJson;
  final List<String> logs;

  VpnState copyWith({
    VpnStatus? status,
    Object? config = _unset,
    Object? exitCountry = _unset,
    Object? allowedPackages = _unset,
    Object? disallowedPackages = _unset,
    Object? errorMessage = _unset,
    Object? lastStatusJson = _unset,
    List<String>? logs,
  }) =>
      VpnState(
        status: status ?? this.status,
        config: identical(config, _unset) ? this.config : config as AppConfig?,
        exitCountry: identical(exitCountry, _unset)
            ? this.exitCountry
            : exitCountry as String?,
        allowedPackages: identical(allowedPackages, _unset)
            ? this.allowedPackages
            : allowedPackages as List<String>?,
        disallowedPackages: identical(disallowedPackages, _unset)
            ? this.disallowedPackages
            : disallowedPackages as List<String>?,
        errorMessage: identical(errorMessage, _unset)
            ? this.errorMessage
            : errorMessage as String?,
        lastStatusJson: identical(lastStatusJson, _unset)
            ? this.lastStatusJson
            : lastStatusJson as Map<String, dynamic>?,
        logs: logs ?? this.logs,
      );
}

final vpnChannelProvider = Provider<VpnChannel>((_) => VpnChannel());
final configServiceProvider = Provider<ConfigService>((_) => ConfigService());

final vpnControllerProvider =
    StateNotifierProvider<VpnController, VpnState>((ref) => VpnController(ref));

class VpnController extends StateNotifier<VpnState> {
  VpnController(this._ref) : super(const VpnState()) {
    _bootstrap();
  }

  final Ref _ref;
  StreamSubscription<Map<String, dynamic>>? _eventsSub;

  Future<void> _bootstrap() async {
    final saved = await _ref.read(configServiceProvider).load();
    if (saved != null) {
      state = state.copyWith(config: saved);
    }
    _eventsSub = _ref.read(vpnChannelProvider).events().listen((event) {
      final kind = event['kind'] as String? ?? '';
      switch (kind) {
        case 'status':
          state = state.copyWith(lastStatusJson: event);
          break;
        case 'log':
          final logs = [...state.logs, event['line'] as String? ?? ''];
          if (logs.length > 500) logs.removeRange(0, logs.length - 500);
          state = state.copyWith(logs: logs);
          break;
        case 'error':
          state = state.copyWith(
            status: VpnStatus.error,
            errorMessage: event['message'] as String?,
          );
          break;
      }
    });
  }

  Future<void> importFromUrl(String url) async {
    final cfg = await _ref.read(configServiceProvider).fetchRemote(url);
    await _ref.read(configServiceProvider).save(cfg);
    state = state.copyWith(config: cfg);
  }

  Future<void> importFromText(String text) async {
    final cfg = await _ref.read(configServiceProvider).parseManual(text);
    await _ref.read(configServiceProvider).save(cfg);
    state = state.copyWith(config: cfg);
  }

  Future<void> connect() async {
    final cfg = state.config;
    if (cfg == null) {
      state = state.copyWith(status: VpnStatus.error, errorMessage: 'No config');
      return;
    }
    state = state.copyWith(status: VpnStatus.connecting);
    try {
      final channel = _ref.read(vpnChannelProvider);
      if (!await channel.hasPermission()) {
        await channel.requestPermission();
      }
      await channel.connect(
        configJson: jsonEncode(cfg.toJson()),
        exitCountry: state.exitCountry,
        allowedPackages: state.allowedPackages,
        disallowedPackages: state.disallowedPackages,
      );
      state = state.copyWith(status: VpnStatus.connected);
    } catch (e) {
      state = state.copyWith(status: VpnStatus.error, errorMessage: e.toString());
    }
  }

  Future<void> disconnect() async {
    state = state.copyWith(status: VpnStatus.disconnecting);
    try {
      await _ref.read(vpnChannelProvider).disconnect();
      state = state.copyWith(status: VpnStatus.disconnected);
    } catch (e) {
      state = state.copyWith(status: VpnStatus.error, errorMessage: e.toString());
    }
  }

  void setCountry(String? code) => state = state.copyWith(exitCountry: code);
  void setAllowedPackages(List<String>? pkgs) =>
      state = state.copyWith(allowedPackages: pkgs);
  void setDisallowedPackages(List<String>? pkgs) =>
      state = state.copyWith(disallowedPackages: pkgs);

  @override
  void dispose() {
    _eventsSub?.cancel();
    super.dispose();
  }
}
