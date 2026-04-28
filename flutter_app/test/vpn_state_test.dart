// Verifies the sentinel-based copyWith on VpnState. The sentinel is the
// single thing standing between us and bugs where `null` means "leave
// alone" instead of "clear" (e.g. country picker → Any, switching per-app
// routing mode, error message reset on reconnect).

import 'package:flutter_test/flutter_test.dart';
import 'package:ff_vpn/state/vpn_state.dart';

void main() {
  group('VpnState.copyWith', () {
    test('leaves nullable fields untouched when not passed', () {
      const base = VpnState(
        status: VpnStatus.connecting,
        exitCountry: 'DE',
        allowedPackages: ['com.foo'],
        errorMessage: 'boom',
      );
      final next = base.copyWith(status: VpnStatus.connected);

      expect(next.status, VpnStatus.connected);
      expect(next.exitCountry, 'DE');
      expect(next.allowedPackages, ['com.foo']);
      expect(next.errorMessage, 'boom');
    });

    test('clears nullable fields when explicitly set to null', () {
      const base = VpnState(
        status: VpnStatus.connected,
        exitCountry: 'DE',
        allowedPackages: ['com.foo'],
        disallowedPackages: ['com.bar'],
        errorMessage: 'boom',
      );
      final next = base.copyWith(
        exitCountry: null,
        allowedPackages: null,
        disallowedPackages: null,
        errorMessage: null,
      );

      expect(next.status, VpnStatus.connected);
      expect(next.exitCountry, isNull);
      expect(next.allowedPackages, isNull);
      expect(next.disallowedPackages, isNull);
      expect(next.errorMessage, isNull);
    });
  });
}
