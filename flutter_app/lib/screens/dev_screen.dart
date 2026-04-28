import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';

import '../state/vpn_state.dart';

class DevScreen extends ConsumerStatefulWidget {
  const DevScreen({super.key});

  @override
  ConsumerState<DevScreen> createState() => _DevScreenState();
}

class _DevScreenState extends ConsumerState<DevScreen> {
  Timer? _poll;
  Map<String, dynamic>? _status;

  @override
  void initState() {
    super.initState();
    _poll = Timer.periodic(const Duration(seconds: 1), (_) async {
      final s = await ref.read(vpnChannelProvider).status();
      if (mounted) setState(() => _status = s);
    });
  }

  @override
  void dispose() {
    _poll?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final logs = ref.watch(vpnControllerProvider).logs;
    return Scaffold(
      appBar: AppBar(title: const Text('Developer')),
      body: Column(
        children: [
          if (_status != null)
            Padding(
              padding: const EdgeInsets.all(12),
              child: Card(
                child: Padding(
                  padding: const EdgeInsets.all(12),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: _status!.entries
                        .map((e) => Text('${e.key}: ${e.value}'))
                        .toList(),
                  ),
                ),
              ),
            ),
          Expanded(
            child: Container(
              color: Colors.black,
              padding: const EdgeInsets.all(8),
              child: ListView.builder(
                itemCount: logs.length,
                itemBuilder: (_, i) => Text(
                  logs[i],
                  style: const TextStyle(
                    fontFamily: 'monospace',
                    color: Colors.greenAccent,
                    fontSize: 11,
                  ),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}
