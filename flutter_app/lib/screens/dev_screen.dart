import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
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
  final _scroll = ScrollController();
  bool _autoScroll = true;
  int _lastLogCount = 0;

  @override
  void initState() {
    super.initState();
    _poll = Timer.periodic(const Duration(seconds: 1), (_) async {
      try {
        final s = await ref.read(vpnChannelProvider).status();
        if (mounted) setState(() => _status = s);
      } catch (_) {
        // status fetch can fail before the service is up — non-fatal
      }
    });
  }

  @override
  void dispose() {
    _poll?.cancel();
    _scroll.dispose();
    super.dispose();
  }

  void _maybeAutoScroll(int newCount) {
    if (!_autoScroll || newCount == _lastLogCount) return;
    _lastLogCount = newCount;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!_scroll.hasClients) return;
      _scroll.jumpTo(_scroll.position.maxScrollExtent);
    });
  }

  @override
  Widget build(BuildContext context) {
    final logs = ref.watch(vpnControllerProvider).logs;
    _maybeAutoScroll(logs.length);
    return Scaffold(
      appBar: AppBar(
        title: const Text('Developer'),
        actions: [
          IconButton(
            tooltip: _autoScroll ? 'Pause auto-scroll' : 'Resume auto-scroll',
            icon: Icon(_autoScroll ? Icons.pause : Icons.play_arrow),
            onPressed: () => setState(() => _autoScroll = !_autoScroll),
          ),
          IconButton(
            tooltip: 'Copy logs',
            icon: const Icon(Icons.copy),
            onPressed: logs.isEmpty
                ? null
                : () async {
                    await Clipboard.setData(
                      ClipboardData(text: logs.join('\n')),
                    );
                    if (context.mounted) {
                      ScaffoldMessenger.of(context).showSnackBar(
                        const SnackBar(
                          content: Text('Logs copied'),
                          duration: Duration(seconds: 1),
                        ),
                      );
                    }
                  },
          ),
          IconButton(
            tooltip: 'Clear logs',
            icon: const Icon(Icons.delete_outline),
            onPressed: logs.isEmpty
                ? null
                : () => ref.read(vpnControllerProvider.notifier).clearLogs(),
          ),
        ],
      ),
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
              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
              child: logs.isEmpty
                  ? const Center(
                      child: Text(
                        'Press Connect to start streaming logs',
                        style: TextStyle(
                          fontFamily: 'monospace',
                          color: Colors.white54,
                          fontSize: 12,
                        ),
                      ),
                    )
                  : ListView.builder(
                      controller: _scroll,
                      itemCount: logs.length,
                      itemBuilder: (_, i) => SelectableText(
                        logs[i],
                        style: TextStyle(
                          fontFamily: 'monospace',
                          color: _colorForLine(logs[i]),
                          fontSize: 11,
                        ),
                      ),
                    ),
            ),
          ),
          Container(
            color: Colors.black87,
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
            width: double.infinity,
            child: Text(
              '${logs.length} lines · auto-scroll ${_autoScroll ? "on" : "off"}',
              style: const TextStyle(
                fontFamily: 'monospace',
                color: Colors.white60,
                fontSize: 10,
              ),
            ),
          ),
        ],
      ),
    );
  }

  Color _colorForLine(String line) {
    // logcat threadtime format: "MM-DD HH:MM:SS.SSS  pid  tid  level  TAG ..."
    // Match the level character; fall back to scanning the whole line.
    final upper = line.toUpperCase();
    if (upper.contains(' E ') || upper.contains('ERROR') || upper.contains(' FATAL')) {
      return Colors.redAccent;
    }
    if (upper.contains(' W ') || upper.contains('WARN')) {
      return Colors.amberAccent;
    }
    if (line.startsWith('[svc]') ||
        line.startsWith('[vpn]') ||
        line.contains(' I ') ||
        upper.contains('INFO')) {
      return Colors.lightGreenAccent;
    }
    return Colors.greenAccent;
  }
}
