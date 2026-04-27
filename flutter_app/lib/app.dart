import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';

import 'screens/country_screen.dart';
import 'screens/dev_screen.dart';
import 'screens/home_screen.dart';
import 'screens/import_screen.dart';
import 'screens/per_app_screen.dart';

class FfVpnApp extends StatefulWidget {
  const FfVpnApp({super.key});

  @override
  State<FfVpnApp> createState() => _FfVpnAppState();
}

class _FfVpnAppState extends State<FfVpnApp> {
  // Created once and stored on the State so framework rebuilds (orientation,
  // theme, locale changes) don't discard the navigation stack.
  late final GoRouter _router = GoRouter(
    initialLocation: '/',
    routes: [
      GoRoute(path: '/', builder: (_, __) => const HomeScreen()),
      GoRoute(path: '/import', builder: (_, __) => const ImportScreen()),
      GoRoute(path: '/dev', builder: (_, __) => const DevScreen()),
      GoRoute(path: '/per-app', builder: (_, __) => const PerAppScreen()),
      GoRoute(path: '/country', builder: (_, __) => const CountryScreen()),
    ],
  );

  @override
  void dispose() {
    _router.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp.router(
      title: 'ff-vpn',
      debugShowCheckedModeBanner: false,
      theme: _buildTheme(Brightness.light),
      darkTheme: _buildTheme(Brightness.dark),
      themeMode: ThemeMode.system,
      routerConfig: _router,
    );
  }

  ThemeData _buildTheme(Brightness brightness) {
    final base = ThemeData(
      useMaterial3: true,
      brightness: brightness,
      colorSchemeSeed: const Color(0xFFF38020),
    );
    return base.copyWith(
      scaffoldBackgroundColor:
          brightness == Brightness.dark ? const Color(0xFF0F0F12) : const Color(0xFFFAFAFB),
      textTheme: base.textTheme.apply(fontFamily: 'Inter'),
      appBarTheme: const AppBarTheme(centerTitle: false, elevation: 0),
    );
  }
}
