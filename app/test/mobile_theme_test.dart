import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:gbf_flash_cache/main.dart';
import 'package:gbf_flash_cache/mobile_dashboard.dart';

void main() {
  testWidgets(
    'Theme saves, restores, follows system, and handles save failure',
    (tester) async {
      tester.view.physicalSize = const Size(800, 1600);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      addTearDown(tester.platformDispatcher.clearPlatformBrightnessTestValue);
      final saved = <String, String>{};
      final writes = <Map>[];
      var fail = false;
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(const MethodChannel('gbf/core'), (
            call,
          ) async {
            if (call.method == 'init') return saved;
            if (call.method == 'settings') {
              if (fail) throw PlatformException(code: 'save', message: '无法保存');
              final args = Map<String, String>.from(call.arguments as Map);
              writes.add(args);
              saved.addAll(args);
            }
            return {'running': 'false'};
          });
      Widget shell() {
        var mode = ThemeMode.dark;
        return StatefulBuilder(
          builder: (context, update) => MaterialApp(
            theme: appTheme(Brightness.light),
            darkTheme: appTheme(Brightness.dark),
            themeMode: mode,
            home: MobileDashboard(
              onThemeChanged: (value) => update(() => mode = value),
            ),
          ),
        );
      }

      Brightness brightness() =>
          Theme.of(tester.element(find.byType(MobileDashboard))).brightness;
      Future<void> choose(String label) async {
        await tester.tap(find.byKey(const ValueKey('theme-menu')));
        await tester.pumpAndSettle();
        await tester.tap(find.widgetWithText(PopupMenuItem<ThemeMode>, label));
        await tester.pumpAndSettle();
      }

      await tester.pumpWidget(shell());
      await tester.pumpAndSettle();
      expect(brightness(), Brightness.dark);
      await tester.tap(find.byTooltip('设置'));
      await tester.pumpAndSettle();
      await choose('亮色');
      expect(brightness(), Brightness.light);
      expect(writes.single, {'themeMode': 'light'});
      await tester.pumpWidget(const SizedBox());
      await tester.pumpWidget(shell());
      await tester.pumpAndSettle();
      expect(brightness(), Brightness.light);
      await tester.tap(find.byTooltip('设置'));
      await tester.pumpAndSettle();
      fail = true;
      await choose('暗色');
      expect(brightness(), Brightness.light);
      expect(saved['themeMode'], 'light');
      fail = false;
      await choose('跟随系统');
      tester.platformDispatcher.platformBrightnessTestValue = Brightness.dark;
      await tester.pumpAndSettle();
      expect(brightness(), Brightness.dark);
      tester.platformDispatcher.platformBrightnessTestValue = Brightness.light;
      await tester.pumpAndSettle();
      expect(brightness(), Brightness.light);
      await choose('暗色');
      expect(brightness(), Brightness.dark);
      await tester.pumpWidget(const SizedBox());
    },
  );
}
