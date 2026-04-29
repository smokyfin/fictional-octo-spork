// Smoke tests for the strict-allowlist VLESS/Reality/gRPC config parser
// in `lib/services/config_service.dart`. Mirrors the Rust-side tests in
// `rust/core/src/config.rs` so the two parsers cannot drift silently.

import 'dart:convert';

import 'package:flutter_test/flutter_test.dart';
import 'package:ff_vpn/services/config_service.dart';

const _validUpstreamConfig = '''
{
  "bridge_rsa_id": "715213AEA5BBE71AB2E9E1AFEE02D0170206021F",
  "bridge_ed25519_id": "Rq4fdFNepS2oTFnyNrQon9FDWi46m5OFZKAGVFmMe9I",
  "doh_server": "https://dns.google/dns-query",
  "doh_server_ip": "8.8.8.8",
  "routing": { "rules": [{ "type": "field", "outboundTag": "blocked" }] },
  "inbounds": [{ "tag": "ignore-me", "protocol": "socks" }],
  "outbounds": [
    {
      "tag": "proxy",
      "protocol": "vless",
      "settings": {
        "vnext": [
          {
            "address": "217.177.47.30",
            "port": 8090,
            "users": [
              { "id": "ea183a5a-a968-4343-b468-7057b1384770", "encryption": "none", "flow": "" }
            ]
          }
        ]
      },
      "streamSettings": {
        "network": "grpc",
        "grpcSettings": { "serviceName": "grpc" },
        "security": "reality",
        "realitySettings": {
          "serverName": "urentbike.ru",
          "publicKey": "aEikAoKpvBlef3fmB956k_I7X3iI2oQ032Zs3YRAiEw",
          "shortId": "6b2f4e6ac9b1d2f0",
          "fingerprint": "qq"
        }
      }
    }
  ]
}
''';

void main() {
  group('AppConfig.parse (upstream Xray/VLESS schema)', () {
    test('extracts only the allowlisted fields', () {
      final cfg = AppConfig.parse(_validUpstreamConfig);

      expect(cfg.bridgeRsaId, '715213AEA5BBE71AB2E9E1AFEE02D0170206021F');
      expect(cfg.bridgeEd25519Id, 'Rq4fdFNepS2oTFnyNrQon9FDWi46m5OFZKAGVFmMe9I');
      expect(cfg.dohServer, 'https://dns.google/dns-query');
      expect(cfg.dohServerIp, '8.8.8.8');

      expect(cfg.outbound.address, '217.177.47.30');
      expect(cfg.outbound.port, 8090);
      expect(cfg.outbound.userId, 'ea183a5a-a968-4343-b468-7057b1384770');
      expect(cfg.outbound.grpcServiceName, 'grpc');

      expect(cfg.outbound.reality.serverName, 'urentbike.ru');
      expect(
        cfg.outbound.reality.publicKey,
        'aEikAoKpvBlef3fmB956k_I7X3iI2oQ032Zs3YRAiEw',
      );
      expect(cfg.outbound.reality.shortId, '6b2f4e6ac9b1d2f0');
      expect(cfg.outbound.reality.fingerprint, 'qq');
    });

    test('rejects non-reality security', () {
      final bad = jsonDecode(_validUpstreamConfig) as Map<String, dynamic>;
      ((bad['outbounds'] as List).first as Map<String, dynamic>)['streamSettings']
          ['security'] = 'tls';
      expect(() => AppConfig.parse(jsonEncode(bad)), throwsFormatException);
    });

    test('rejects networks other than grpc / xhttp', () {
      final bad = jsonDecode(_validUpstreamConfig) as Map<String, dynamic>;
      ((bad['outbounds'] as List).first as Map<String, dynamic>)['streamSettings']
          ['network'] = 'tcp';
      expect(() => AppConfig.parse(jsonEncode(bad)), throwsFormatException);
    });

    test('parses xhttp config without grpcSettings', () {
      const xhttpConfig = '''
      {
        "bridge_rsa_id": "9A5E28708880EB92217A937F56D640D23551F886",
        "bridge_ed25519_id": "1Vvw08iKSZVW9ghoiYBWl7qR30d5DNJu0c7EFp0XFZ4",
        "doh_server": "https://dns.google/dns-query",
        "outbounds": [{
          "tag": "proxy",
          "protocol": "vless",
          "settings": {"vnext":[{"address":"144.31.184.170","port":8090,
            "users":[{"id":"3701ba53-4573-466c-a474-f37923ce5bd1","encryption":"none","flow":""}]}]},
          "streamSettings": {"network":"xhttp",
            "security":"reality","realitySettings":{"serverName":"ads.x5.ru",
              "publicKey":"94T5KqTcnBNXDmlobpF7rmsYmPt6vqB_dWQcIi9XjAI",
              "shortId":"6b2f4e6ac9b1d2f0","fingerprint":"qq"}}
        }]
      }
      ''';
      final cfg = AppConfig.parse(xhttpConfig);
      expect(cfg.outbound.network, 'xhttp');
      expect(cfg.outbound.grpcServiceName, '');
      expect(cfg.outbound.address, '144.31.184.170');
      expect(cfg.outbound.reality.serverName, 'ads.x5.ru');
    });

    test('rejects when no vless outbound exists', () {
      const onlyTrojan = '''
      {
        "bridge_rsa_id": "x", "bridge_ed25519_id": "y",
        "doh_server": "https://dns.google/dns-query",
        "outbounds": [{
          "tag": "t", "protocol": "trojan",
          "settings": { "vnext": [] },
          "streamSettings": { "network": "grpc", "security": "reality" }
        }]
      }
      ''';
      expect(() => AppConfig.parse(onlyTrojan), throwsFormatException);
    });
  });

  group('AppConfig <-> toJson/fromJson round-trip', () {
    test('round-trips through the flat schema used for on-disk persistence', () {
      final original = AppConfig.parse(_validUpstreamConfig);
      final round = AppConfig.fromJson(original.toJson());

      expect(round.bridgeRsaId, original.bridgeRsaId);
      expect(round.bridgeEd25519Id, original.bridgeEd25519Id);
      expect(round.dohServer, original.dohServer);
      expect(round.dohServerIp, original.dohServerIp);

      expect(round.outbound.address, original.outbound.address);
      expect(round.outbound.port, original.outbound.port);
      expect(round.outbound.userId, original.outbound.userId);
      expect(round.outbound.network, original.outbound.network);
      expect(round.outbound.grpcServiceName, original.outbound.grpcServiceName);

      expect(round.outbound.reality.serverName, original.outbound.reality.serverName);
      expect(round.outbound.reality.publicKey, original.outbound.reality.publicKey);
      expect(round.outbound.reality.shortId, original.outbound.reality.shortId);
      expect(round.outbound.reality.fingerprint, original.outbound.reality.fingerprint);
    });
  });
}
