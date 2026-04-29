import 'dart:convert';
import 'dart:io';

import 'package:http/http.dart' as http;
import 'package:path_provider/path_provider.dart';
import 'package:shared_preferences/shared_preferences.dart';

const defaultRemoteConfigUrl = 'https://incss.ru/vless.conf';

/// Strict allowlist parser mirroring the Rust core. Only `bridge_rsa_id`,
/// `bridge_ed25519_id`, `doh_server`, `doh_server_ip`, and the first VLESS
/// outbound are extracted; everything else is dropped.
class AppConfig {
  AppConfig({
    required this.bridgeRsaId,
    required this.bridgeEd25519Id,
    required this.dohServer,
    required this.dohServerIp,
    required this.skipArti,
    required this.outbound,
  });

  final String bridgeRsaId;
  final String bridgeEd25519Id;
  final String dohServer;
  final String? dohServerIp;
  /// Bypass Arti when true — i.e. route TUN → hev → xray → VLESS.
  /// The UI can override this via `VpnState.skipArtiOverride`.
  final bool skipArti;
  final VlessOutbound outbound;

  Map<String, dynamic> toJson() => {
        'bridge_rsa_id': bridgeRsaId,
        'bridge_ed25519_id': bridgeEd25519Id,
        'doh_server': dohServer,
        'doh_server_ip': dohServerIp,
        'skip_arti': skipArti,
        'outbound': outbound.toJson(),
      };

  /// Inverse of [toJson] — used by [ConfigService.load] to rehydrate a
  /// previously-saved config. Distinct from [parse] which expects the
  /// upstream Xray/VLESS JSON format with `outbounds`, `streamSettings`,
  /// `vnext`, etc.
  factory AppConfig.fromJson(Map<String, dynamic> json) {
    final ob = json['outbound'] as Map<String, dynamic>;
    final reality = ob['reality'] as Map<String, dynamic>;
    return AppConfig(
      bridgeRsaId: json['bridge_rsa_id'] as String,
      bridgeEd25519Id: json['bridge_ed25519_id'] as String,
      dohServer: json['doh_server'] as String,
      dohServerIp: json['doh_server_ip'] as String?,
      skipArti: (json['skip_arti'] ?? false) as bool,
      outbound: VlessOutbound(
        tag: ob['tag'] as String,
        address: ob['address'] as String,
        port: ob['port'] as int,
        userId: ob['user_id'] as String,
        flow: (ob['flow'] ?? '') as String,
        // Older persisted configs predate the `network` field — default to
        // grpc to keep them parseable.
        network: (ob['network'] ?? 'grpc') as String,
        grpcServiceName: (ob['grpc_service_name'] ?? '') as String,
        reality: RealitySettings(
          serverName: reality['server_name'] as String,
          publicKey: reality['public_key'] as String,
          shortId: reality['short_id'] as String,
          fingerprint: reality['fingerprint'] as String,
        ),
      ),
    );
  }

  static AppConfig parse(String body) {
    final raw = jsonDecode(body) as Map<String, dynamic>;
    final outbounds = (raw['outbounds'] as List).cast<Map<String, dynamic>>();
    final vless = outbounds.firstWhere(
      (o) => (o['protocol'] as String).toLowerCase() == 'vless',
      orElse: () => throw const FormatException('no vless outbound'),
    );
    final stream = vless['streamSettings'] as Map<String, dynamic>;
    if ((stream['security'] as String).toLowerCase() != 'reality') {
      throw const FormatException('only reality is supported');
    }
    final network = (stream['network'] as String).toLowerCase();
    if (network != 'grpc' && network != 'xhttp') {
      throw FormatException(
        'only grpc/xhttp networks are supported (got $network)',
      );
    }
    // gRPC carries `serviceName`; xhttp has no analogous identifier.
    final grpcServiceName = network == 'grpc'
        ? (stream['grpcSettings'] as Map<String, dynamic>)['serviceName'] as String
        : '';
    final reality = stream['realitySettings'] as Map<String, dynamic>;
    final vnext = (vless['settings']['vnext'] as List).first as Map<String, dynamic>;
    final user = (vnext['users'] as List).first as Map<String, dynamic>;
    return AppConfig(
      bridgeRsaId: raw['bridge_rsa_id'] as String,
      bridgeEd25519Id: raw['bridge_ed25519_id'] as String,
      dohServer: raw['doh_server'] as String,
      dohServerIp: raw['doh_server_ip'] as String?,
      skipArti: (raw['skip_arti'] ?? false) as bool,
      outbound: VlessOutbound(
        tag: (vless['tag'] ?? 'proxy') as String,
        address: vnext['address'] as String,
        port: vnext['port'] as int,
        userId: user['id'] as String,
        flow: (user['flow'] ?? '') as String,
        network: network,
        grpcServiceName: grpcServiceName,
        reality: RealitySettings(
          serverName: reality['serverName'] as String,
          publicKey: reality['publicKey'] as String,
          shortId: reality['shortId'] as String,
          fingerprint: reality['fingerprint'] as String,
        ),
      ),
    );
  }
}

class VlessOutbound {
  VlessOutbound({
    required this.tag,
    required this.address,
    required this.port,
    required this.userId,
    required this.flow,
    required this.network,
    required this.grpcServiceName,
    required this.reality,
  });

  final String tag;
  final String address;
  final int port;
  final String userId;
  final String flow;
  /// VLESS stream transport: `grpc` or `xhttp`.
  final String network;
  /// Empty string when [network] is `xhttp`.
  final String grpcServiceName;
  final RealitySettings reality;

  Map<String, dynamic> toJson() => {
        'tag': tag,
        'address': address,
        'port': port,
        'user_id': userId,
        'flow': flow,
        'network': network,
        'grpc_service_name': grpcServiceName,
        'reality': reality.toJson(),
      };
}

class RealitySettings {
  RealitySettings({
    required this.serverName,
    required this.publicKey,
    required this.shortId,
    required this.fingerprint,
  });

  final String serverName;
  final String publicKey;
  final String shortId;
  final String fingerprint;

  Map<String, dynamic> toJson() => {
        'server_name': serverName,
        'public_key': publicKey,
        'short_id': shortId,
        'fingerprint': fingerprint,
      };
}

class ConfigService {
  static const _kStoredConfig = 'ff_vpn.config_json';

  Future<AppConfig> fetchRemote(String url) async {
    final resp = await http.get(Uri.parse(url));
    if (resp.statusCode != 200) {
      throw Exception('HTTP ${resp.statusCode} fetching $url');
    }
    return AppConfig.parse(resp.body);
  }

  Future<AppConfig> parseManual(String text) async => AppConfig.parse(text);

  Future<void> save(AppConfig cfg) async {
    final json = jsonEncode(cfg.toJson());
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString(_kStoredConfig, json);
    // Also persist a copy in the private docs directory for the Rust core
    // to load if it ever needs to start without UI input.
    final dir = await getApplicationDocumentsDirectory();
    await dir.create(recursive: true);
    final file = File('${dir.path}/config.json');
    await file.writeAsString(json, flush: true);
  }

  Future<AppConfig?> load() async {
    final prefs = await SharedPreferences.getInstance();
    final json = prefs.getString(_kStoredConfig);
    if (json == null) return null;
    // Use fromJson, NOT parse() — the saved blob is the flat shape produced
    // by toJson(), not the upstream Xray/VLESS schema.
    return AppConfig.fromJson(jsonDecode(json) as Map<String, dynamic>);
  }
}
