import 'package:flutter/material.dart';
import 'package:flutter_animate/flutter_animate.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:go_router/go_router.dart';

import '../state/vpn_state.dart';

class HomeScreen extends ConsumerWidget {
  const HomeScreen({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final state = ref.watch(vpnControllerProvider);
    final ctrl = ref.read(vpnControllerProvider.notifier);
    return Scaffold(
      appBar: AppBar(
        title: const Text('ff-vpn'),
        actions: [
          IconButton(icon: const Icon(Icons.code), onPressed: () => context.push('/dev')),
          IconButton(icon: const Icon(Icons.public), onPressed: () => context.push('/country')),
          IconButton(icon: const Icon(Icons.apps), onPressed: () => context.push('/per-app')),
          IconButton(icon: const Icon(Icons.qr_code_scanner), onPressed: () => context.push('/import')),
        ],
      ),
      body: Center(
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            _StatusOrb(status: state.status),
            const SizedBox(height: 24),
            Text(_label(state), style: Theme.of(context).textTheme.titleMedium),
            const SizedBox(height: 8),
            if (state.config != null)
              Text(
                '${state.config!.outbound.address}:${state.config!.outbound.port}',
                style: Theme.of(context).textTheme.bodySmall,
              )
            else
              TextButton.icon(
                icon: const Icon(Icons.download),
                label: const Text('Import config'),
                onPressed: () => context.push('/import'),
              ),
            const SizedBox(height: 24),
            // Skip Arti toggle. Pipeline default (off) is
            //   TUN -> hev -> Arti -> xray-PT -> VLESS -> server
            // turning it on bypasses Arti:
            //   TUN -> hev -> xray -> VLESS -> server
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 32),
              child: SwitchListTile(
                // Three-state UI collapsed to two for now:
                // null/false → Arti enabled, true → Arti bypass.
                value: state.skipArtiOverride == true,
                onChanged: state.status == VpnStatus.disconnected ||
                        state.status == VpnStatus.error
                    ? (v) => ctrl.setSkipArtiOverride(v)
                    : null,
                title: const Text('Skip Arti'),
                subtitle: const Text(
                  'Off: TUN -> hev -> Arti -> xray -> VLESS. '
                  'On: TUN -> hev -> xray -> VLESS (bypass Tor).',
                ),
                contentPadding: EdgeInsets.zero,
              ),
            ),
            const SizedBox(height: 16),
            FilledButton.tonal(
              onPressed: state.config == null
                  ? null
                  : () {
                      if (state.status == VpnStatus.connected ||
                          state.status == VpnStatus.connecting) {
                        ctrl.disconnect();
                      } else {
                        ctrl.connect();
                      }
                    },
              style: FilledButton.styleFrom(
                padding: const EdgeInsets.symmetric(horizontal: 64, vertical: 16),
                shape: const StadiumBorder(),
              ),
              child: Text(
                state.status == VpnStatus.connected
                    ? 'Disconnect'
                    : state.status == VpnStatus.connecting
                        ? 'Connecting…'
                        : 'Connect',
              ),
            ),
            if (state.errorMessage != null) ...[
              const SizedBox(height: 16),
              Text(state.errorMessage!, style: const TextStyle(color: Colors.redAccent)),
            ],
          ],
        ),
      ),
    );
  }

  String _label(VpnState s) {
    switch (s.status) {
      case VpnStatus.disconnected:
        return 'Disconnected';
      case VpnStatus.connecting:
        return s.skipArtiOverride == true
            ? 'Starting Xray…'
            : 'Bootstrapping Arti + Xray…';
      case VpnStatus.connected:
        return s.skipArtiOverride == true
            ? 'Connected (skip Arti)'
            : 'Connected via Arti + VLESS';
      case VpnStatus.disconnecting:
        return 'Tearing down…';
      case VpnStatus.error:
        return 'Error';
    }
  }
}

class _StatusOrb extends StatelessWidget {
  const _StatusOrb({required this.status});
  final VpnStatus status;

  @override
  Widget build(BuildContext context) {
    final color = switch (status) {
      VpnStatus.connected => const Color(0xFF34C759),
      VpnStatus.connecting => const Color(0xFFF38020),
      VpnStatus.disconnecting => Colors.amber,
      VpnStatus.error => Colors.redAccent,
      VpnStatus.disconnected => Colors.grey.shade500,
    };
    return Container(
      width: 160,
      height: 160,
      decoration: BoxDecoration(
        shape: BoxShape.circle,
        gradient: RadialGradient(
          colors: [color.withValues(alpha: 0.9), color.withValues(alpha: 0.0)],
          radius: 0.8,
        ),
      ),
      child: Center(
        child: Container(
          width: 96,
          height: 96,
          decoration: BoxDecoration(
            color: color,
            shape: BoxShape.circle,
            boxShadow: [
              BoxShadow(color: color.withValues(alpha: 0.55), blurRadius: 28, spreadRadius: 4),
            ],
          ),
          child: const Icon(Icons.shield_moon, color: Colors.white, size: 36),
        ).animate(onPlay: (c) => c.repeat()).scaleXY(
              begin: 1.0,
              end: status == VpnStatus.connecting ? 1.08 : 1.0,
              duration: 900.ms,
              curve: Curves.easeInOut,
            ),
      ),
    );
  }
}
