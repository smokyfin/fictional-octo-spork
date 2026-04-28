import 'package:flutter/material.dart';
import 'package:flutter_riverpod/flutter_riverpod.dart';
import 'package:mobile_scanner/mobile_scanner.dart';

import '../services/config_service.dart';
import '../state/vpn_state.dart';

class ImportScreen extends ConsumerStatefulWidget {
  const ImportScreen({super.key});

  @override
  ConsumerState<ImportScreen> createState() => _ImportScreenState();
}

class _ImportScreenState extends ConsumerState<ImportScreen>
    with SingleTickerProviderStateMixin {
  late final TabController _tabs = TabController(length: 3, vsync: this);
  final _urlCtrl = TextEditingController(text: defaultRemoteConfigUrl);
  final _textCtrl = TextEditingController();
  bool _busy = false;
  String? _error;

  @override
  void dispose() {
    _tabs.dispose();
    _urlCtrl.dispose();
    _textCtrl.dispose();
    super.dispose();
  }

  Future<void> _runImport(Future<void> Function() body) async {
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await body();
      if (mounted) Navigator.of(context).maybePop();
    } catch (e) {
      setState(() => _error = e.toString());
    } finally {
      if (mounted) setState(() => _busy = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final ctrl = ref.read(vpnControllerProvider.notifier);
    return Scaffold(
      appBar: AppBar(
        title: const Text('Import configuration'),
        bottom: TabBar(
          controller: _tabs,
          tabs: const [
            Tab(icon: Icon(Icons.link), text: 'URL'),
            Tab(icon: Icon(Icons.qr_code), text: 'QR'),
            Tab(icon: Icon(Icons.text_fields), text: 'Text'),
          ],
        ),
      ),
      body: Column(
        children: [
          if (_error != null)
            Padding(
              padding: const EdgeInsets.all(8.0),
              child: Text(_error!, style: const TextStyle(color: Colors.redAccent)),
            ),
          Expanded(
            child: TabBarView(
              controller: _tabs,
              children: [
                _UrlTab(
                  controller: _urlCtrl,
                  busy: _busy,
                  onImport: () =>
                      _runImport(() => ctrl.importFromUrl(_urlCtrl.text.trim())),
                ),
                _QrTab(
                  busy: _busy,
                  onScanned: (text) =>
                      _runImport(() => ctrl.importFromText(text)),
                ),
                _TextTab(
                  controller: _textCtrl,
                  busy: _busy,
                  onImport: () =>
                      _runImport(() => ctrl.importFromText(_textCtrl.text)),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _UrlTab extends StatelessWidget {
  const _UrlTab(
      {required this.controller, required this.busy, required this.onImport});
  final TextEditingController controller;
  final bool busy;
  final VoidCallback onImport;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.all(16),
      child: Column(
        children: [
          TextField(
            controller: controller,
            decoration: const InputDecoration(
              labelText: 'Configuration URL',
              border: OutlineInputBorder(),
            ),
          ),
          const SizedBox(height: 16),
          FilledButton.icon(
            onPressed: busy ? null : onImport,
            icon: const Icon(Icons.cloud_download),
            label: const Text('Import'),
          ),
        ],
      ),
    );
  }
}

class _QrTab extends StatelessWidget {
  const _QrTab({required this.busy, required this.onScanned});
  final bool busy;
  final ValueChanged<String> onScanned;

  @override
  Widget build(BuildContext context) {
    return MobileScanner(
      onDetect: (capture) {
        if (busy) return;
        for (final b in capture.barcodes) {
          if (b.rawValue != null) {
            onScanned(b.rawValue!);
            return;
          }
        }
      },
    );
  }
}

class _TextTab extends StatelessWidget {
  const _TextTab(
      {required this.controller, required this.busy, required this.onImport});
  final TextEditingController controller;
  final bool busy;
  final VoidCallback onImport;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.all(16),
      child: Column(
        children: [
          Expanded(
            child: TextField(
              controller: controller,
              maxLines: null,
              expands: true,
              decoration: const InputDecoration(
                hintText: 'Paste JSON configuration here',
                border: OutlineInputBorder(),
              ),
            ),
          ),
          const SizedBox(height: 16),
          FilledButton.icon(
            onPressed: busy ? null : onImport,
            icon: const Icon(Icons.check),
            label: const Text('Import'),
          ),
        ],
      ),
    );
  }
}
