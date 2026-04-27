import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:installed_apps/installed_apps.dart';
import 'package:installed_apps/app_info.dart';

import '../state/vpn_state.dart';

class PerAppScreen extends ConsumerStatefulWidget {
  const PerAppScreen({super.key});

  @override
  ConsumerState<PerAppScreen> createState() => _PerAppScreenState();
}

class _PerAppScreenState extends ConsumerState<PerAppScreen> {
  late Future<List<AppInfo>> _apps;
  bool _useAllowlist = false;
  final Set<String> _selected = <String>{};

  @override
  void initState() {
    super.initState();
    _apps = _loadApps();
    final st = ref.read(vpnControllerProvider);
    if (st.allowedPackages != null) {
      _useAllowlist = true;
      _selected.addAll(st.allowedPackages!);
    } else if (st.disallowedPackages != null) {
      _useAllowlist = false;
      _selected.addAll(st.disallowedPackages!);
    }
  }

  Future<List<AppInfo>> _loadApps() async {
    if (defaultTargetPlatform != TargetPlatform.android) return [];
    return InstalledApps.getInstalledApps(true, true);
  }

  @override
  Widget build(BuildContext context) {
    if (defaultTargetPlatform != TargetPlatform.android) {
      return Scaffold(
        appBar: AppBar(title: const Text('Per-app routing')),
        body: const Center(
          child: Text('Per-app routing is only available on Android.'),
        ),
      );
    }
    return Scaffold(
      appBar: AppBar(
        title: const Text('Per-app routing'),
        actions: [
          IconButton(
            icon: const Icon(Icons.save),
            onPressed: () {
              final ctrl = ref.read(vpnControllerProvider.notifier);
              if (_useAllowlist) {
                ctrl.setAllowedPackages(_selected.toList());
                ctrl.setDisallowedPackages(null);
              } else {
                ctrl.setDisallowedPackages(_selected.toList());
                ctrl.setAllowedPackages(null);
              }
              Navigator.of(context).maybePop();
            },
          ),
        ],
      ),
      body: Column(
        children: [
          SwitchListTile(
            title: Text(_useAllowlist
                ? 'Allowlist: only checked apps go through VPN'
                : 'Blocklist: checked apps bypass VPN'),
            value: _useAllowlist,
            onChanged: (v) => setState(() => _useAllowlist = v),
          ),
          const Divider(height: 1),
          Expanded(
            child: FutureBuilder<List<AppInfo>>(
              future: _apps,
              builder: (_, snap) {
                if (!snap.hasData) {
                  return const Center(child: CircularProgressIndicator());
                }
                final apps = snap.data!;
                return ListView.builder(
                  itemCount: apps.length,
                  itemBuilder: (_, i) {
                    final app = apps[i];
                    final pkg = app.packageName;
                    final selected = _selected.contains(pkg);
                    return CheckboxListTile(
                      value: selected,
                      title: Text(app.name),
                      subtitle: Text(pkg),
                      onChanged: (v) => setState(() {
                        if (v == true) {
                          _selected.add(pkg);
                        } else {
                          _selected.remove(pkg);
                        }
                      }),
                    );
                  },
                );
              },
            ),
          ),
        ],
      ),
    );
  }
}
