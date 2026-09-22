import 'dart:async';
import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:gbf_flash_cache/mobile_dashboard.dart';
import 'package:gbf_flash_cache/mobile_proxy.dart';

void main() {
  testWidgets(
    'Scope opens without app scan; a slow scan is cancellable and reused',
    (tester) async {
      var scans = 0;
      final scan = Completer<Map<String, String>>();
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(const MethodChannel('gbf/core'), (
            call,
          ) async {
            if (call.method == 'proxy_apps') {
              scans++;
              return scan.future;
            }
            return <String, String>{};
          });
      await tester.pumpWidget(
        MaterialApp(home: const MobileProxySettings(active: false)),
      );
      await tester.pumpAndSettle();
      expect(scans, 0);
      await tester.tap(find.text('包含应用').last);
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 400));
      expect(scans, 1);
      await tester.tap(find.byTooltip('返回').last);
      await tester.pumpAndSettle();
      expect(find.text('代理应用范围'), findsOneWidget);
      scan.complete({'apps': '[]'});
      await tester.pumpAndSettle();
      await tester.tap(find.text('排除应用').last);
      await tester.pumpAndSettle();
      expect(scans, 1);
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets('Scope saves immediately and rolls back if persistence fails', (
    tester,
  ) async {
    var reject = true;
    final writes = <Map>[];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(const MethodChannel('gbf/core'), (
          call,
        ) async {
          if (call.method == 'proxy_save') {
            writes.add(Map.of(call.arguments as Map));
            if (reject) throw PlatformException(code: 'save', message: '保存失败');
          }
          return <String, String>{};
        });
    await tester.pumpWidget(
      MaterialApp(home: const MobileProxySettings(active: false)),
    );
    await tester.pumpAndSettle();
    Future<void> exclude() async {
      await tester.tap(find.byType(DropdownButtonFormField<String>));
      await tester.pumpAndSettle();
      await tester.tap(find.text('排除应用').last);
      await tester.pumpAndSettle();
    }

    await exclude();
    expect(writes.single['mode'], 'exclude');
    expect(find.text('保存失败'), findsOneWidget);
    expect(find.text('两份名单均不生效。'), findsOneWidget);
    reject = false;
    await exclude();
    expect(find.text('仅排除名单生效。'), findsOneWidget);
    expect(find.text('保存'), findsNothing);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets(
    'All scope preserves independent lists; selection does not subtract',
    (tester) async {
      tester.view.physicalSize = const Size(360, 800);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      Map? saved;
      var reject = true;
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(const MethodChannel('gbf/core'), (
            call,
          ) async {
            if (call.method == 'proxy_settings') {
              return {
                'mode': 'all',
                'included': '["browser"]',
                'excluded': '["browser","upstream"]',
              };
            }
            if (call.method == 'proxy_apps') {
              return {
                'apps': '[{"package":"browser","label":"Browser"},{"package":"upstream","label":"CMFA"}]',
              };
            }
            if (call.method == 'proxy_save') {
              if (reject) {
                throw PlatformException(code: 'save', message: '名单保存失败');
              }
              saved = call.arguments as Map;
            }
            return <String, String>{};
          });
      await tester.pumpWidget(
        MaterialApp(
          theme: ThemeData.dark(),
          home: const MobileProxySettings(active: false),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text('两份名单均不生效。'), findsOneWidget);
      expect(find.text('上游代理应用'), findsNothing);
      await tester.tap(find.text('包含应用').last);
      await tester.pumpAndSettle();
      await tester.tap(find.text('CMFA'));
      await tester.pumpAndSettle();
      expect(find.text('名单保存失败'), findsOneWidget);
      expect(saved, isNull);
      expect(
        tester
            .widget<CheckboxListTile>(
              find.widgetWithText(CheckboxListTile, 'CMFA'),
            )
            .value,
        false,
      );
      reject = false;
      await tester.tap(find.text('CMFA'));
      await tester.pumpAndSettle();
      expect(find.text('保存'), findsNothing);
      expect(saved!['mode'], 'all');
      expect(saved!.containsKey('upstreamApp'), isFalse);
      expect(jsonDecode(saved!['included'] as String), ['browser', 'upstream']);
      expect(jsonDecode(saved!['excluded'] as String), ['browser', 'upstream']);
      expect(tester.takeException(), isNull);
    },
  );
  testWidgets(
    'Proxy configuration does not start service; denied start rolls cache back',
    (tester) async {
      var running = false;
      final calls = <String>[];
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(const MethodChannel('gbf/core'), (
            call,
          ) async {
            calls.add(call.method);
            if (call.method == 'start') running = true;
            if (call.method == 'stop') running = false;
            if (call.method == 'proxy_start') {
              throw PlatformException(code: 'cancelled');
            }
            return {'running': '$running', 'systemProxy': 'false'};
          });
      await tester.pumpWidget(
        MaterialApp(theme: ThemeData.dark(), home: const MobileDashboard()),
      );
      await tester.pumpAndSettle();
      await tester.scrollUntilVisible(find.text('系统代理'), 300);
      await tester.ensureVisible(find.text('系统代理'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('系统代理'));
      await tester.pumpAndSettle();
      expect(calls, contains('proxy_save'));
      expect(calls, isNot(contains('start')));
      await tester.scrollUntilVisible(find.text('启动'), -300);
      await tester.tap(find.text('启动'));
      await tester.pumpAndSettle();
      expect(calls.indexOf('start'), lessThan(calls.indexOf('proxy_start')));
      expect(running, false);
      expect(calls, contains('stop'));
      await tester.scrollUntilVisible(find.text('系统代理'), 300);
      expect(
        tester
            .widget<SwitchListTile>(find.widgetWithText(SwitchListTile, '系统代理'))
            .value,
        true,
      );
      await tester.pumpWidget(const SizedBox());
    },
  );
  testWidgets(
    'Saved proxy option is restored and starts and stops with service',
    (tester) async {
      var running = false;
      final calls = <String>[];
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(const MethodChannel('gbf/core'), (
            call,
          ) async {
            calls.add(call.method);
            if (call.method == 'init') return {'systemProxyEnabled': 'true'};
            if (call.method == 'start') running = true;
            if (call.method == 'stop') running = false;
            return {'running': '$running'};
          });
      await tester.pumpWidget(const MaterialApp(home: MobileDashboard()));
      await tester.pumpAndSettle();
      expect(calls, isNot(contains('start')));
      await tester.tap(find.text('启动'));
      await tester.pumpAndSettle();
      expect(calls.where((c) => c == 'start' || c == 'proxy_start').toList(), [
        'start',
        'proxy_start',
      ]);
      await tester.tap(find.text('停止'));
      await tester.pumpAndSettle();
      expect(calls, contains('stop'));
      await tester.scrollUntilVisible(find.text('系统代理'), 300);
      expect(
        tester
            .widget<SwitchListTile>(find.widgetWithText(SwitchListTile, '系统代理'))
            .value,
        true,
      );
      await tester.pumpWidget(const SizedBox());
    },
  );
  testWidgets(
    'Include mode can be selected empty and its last app deselected',
    (tester) async {
      var saved = <String, String>{
        'mode': 'all',
        'included': '[]',
        'excluded': '["upstream"]',
      };
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(const MethodChannel('gbf/core'), (
            call,
          ) async {
            if (call.method == 'proxy_settings') return saved;
            if (call.method == 'proxy_apps') {
              return {'apps': '[{"package":"browser","label":"Browser"}]'};
            }
            if (call.method == 'proxy_save') {
              saved = Map<String, String>.from(call.arguments as Map);
            }
            return <String, String>{};
          });
      await tester.pumpWidget(
        const MaterialApp(home: MobileProxySettings(active: false)),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.byType(DropdownButtonFormField<String>));
      await tester.pumpAndSettle();
      await tester.tap(find.text('包含应用').last);
      await tester.pumpAndSettle();
      expect(saved['mode'], 'include');
      expect(saved['included'], '[]');
      await tester.tap(find.widgetWithText(ListTile, '包含应用'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Browser'));
      await tester.pumpAndSettle();
      expect(jsonDecode(saved['included']!), ['browser']);
      await tester.tap(find.text('Browser'));
      await tester.pumpAndSettle();
      expect(saved['included'], '[]');
      expect(saved['excluded'], '["upstream"]');
      expect(
        tester.widget<CheckboxListTile>(find.byType(CheckboxListTile)).value,
        false,
      );
      await tester.tap(find.byTooltip('返回').last);
      await tester.pumpAndSettle();
      await tester.tap(find.widgetWithText(ListTile, '包含应用'));
      await tester.pumpAndSettle();
      expect(
        tester.widget<CheckboxListTile>(find.byType(CheckboxListTile)).value,
        false,
      );
      expect(tester.takeException(), isNull);
    },
  );
}
