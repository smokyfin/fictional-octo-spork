import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../state/vpn_state.dart';

const _countries = <(String, String)>[
  ('', 'Any'),
  ('AT', 'Austria'),
  ('CH', 'Switzerland'),
  ('CZ', 'Czechia'),
  ('DE', 'Germany'),
  ('ES', 'Spain'),
  ('FI', 'Finland'),
  ('FR', 'France'),
  ('GB', 'United Kingdom'),
  ('IE', 'Ireland'),
  ('IS', 'Iceland'),
  ('JP', 'Japan'),
  ('NL', 'Netherlands'),
  ('NO', 'Norway'),
  ('PL', 'Poland'),
  ('SE', 'Sweden'),
  ('SG', 'Singapore'),
  ('US', 'United States'),
];

class CountryScreen extends ConsumerWidget {
  const CountryScreen({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final selected = ref.watch(vpnControllerProvider).exitCountry ?? '';
    final ctrl = ref.read(vpnControllerProvider.notifier);
    return Scaffold(
      appBar: AppBar(title: const Text('Tor exit country')),
      body: ListView.builder(
        itemCount: _countries.length,
        itemBuilder: (_, i) {
          final (code, label) = _countries[i];
          return RadioListTile<String>(
            value: code,
            groupValue: selected,
            title: Text(label),
            subtitle: code.isEmpty ? null : Text(code),
            onChanged: (v) {
              ctrl.setCountry(v == null || v.isEmpty ? null : v);
              Navigator.of(context).maybePop();
            },
          );
        },
      ),
    );
  }
}
