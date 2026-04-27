import 'dart:convert';

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
    required this.outbound,
  });

  final String bridgeRsaId;
  final String bridgeEd25519Id;
  final String dohServer;
  final String? dohServerIp;
  final VlessOutbound outbound;

  Map<String, dynamic> toJson() => {
        'bridge_rsa_id': bridgeRsaId,
        'bridge_ed25519_id': bridgeEd25519Id,
        'doh_server': dohServer,
        'doh_server_ip': dohServerIp,
        'outbound': outbound.toJson(),
      };

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
    if ((stream['network'] as String).toLowerCase() != 'grpc') {
      throw const FormatException('only grpc network is supported');
    }
    final grpc = stream['grpcSettings'] as Map<String, dynamic>;
    final reality = stream['realitySettings'] as Map<String, dynamic>;
    final vnext = (vless['settings']['vnext'] as List).first as Map<String, dynamic>;
    final user = (vnext['users'] as List).first as Map<String, dynamic>;
    return AppConfig(
      bridgeRsaId: raw['bridge_rsa_id'] as String,
      bridgeEd25519Id: raw['bridge_ed25519_id'] as String,
      dohServer: raw['doh_server'] as String,
      dohServerIp: raw['doh_server_ip'] as String?,
      outbound: VlessOutbound(
        tag: (vless['tag'] ?? 'proxy') as String,
        address: vnext['address'] as String,
        port: vnext['port'] as int,
        userId: user['id'] as String,
        flow: (user['flow'] ?? '') as String,
        grpcServiceName: grpc['serviceName'] as String,
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
    required this.grpcServiceName,
    required this.reality,
  });

  final String tag;
  final String address;
  final int port;
  final String userId;
  final String flow;
  final String grpcServiceName;
  final RealitySettings reality;

  Map<String, dynamic> toJson() => {
        'tag': tag,
        'address': address,
        'port': port,
        'user_id': userId,
        'flow': flow,
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
    final prefs = await SharedPreferences.getInstance();
    await prefs.setString(_kStoredConfig, jsonEncode(cfg.toJson()));
    // Also persist a copy in the private docs directory for the Rust core
    // to load if it ever needs to start without UI input.
    final dir = await getApplicationDocumentsDirectory();
    final f = await dir.create(recursive: true);
    await (f).resolve('config.json');
  }

  Future<AppConfig?> load() async {
    final prefs = await SharedPreferences.getInstance();
    final json = prefs.getString(_kStoredConfig);
    if (json == null) return null;
    return AppConfig.parse(json);
  }
}
