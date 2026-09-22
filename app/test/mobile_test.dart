import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:gbf_flash_cache/mobile_dashboard.dart';

void main() {
  testWidgets('Mobile saves and starts with exact proxy usernames', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(800, 1600);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final writes = <Map>[], starts = <Map>[];
    var running = false;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(const MethodChannel('gbf/core'), (
          call,
        ) async {
          if (call.method == 'init') {
            return {
              'proxy': 'true',
              'host': '127.0.0.1',
              'proxyPort': '1080',
              'username': ' initial ',
              'password': ' secret ',
            };
          }
          if (call.method == 'settings') {
            writes.add(Map.of(call.arguments as Map));
          }
          if (call.method == 'start') {
            starts.add(Map.of(call.arguments as Map));
            running = true;
          }
          if (call.method == 'stop') running = false;
          return {'running': '$running'};
        });
    await tester.pumpWidget(const MaterialApp(home: MobileDashboard()));
    await tester.pumpAndSettle();
    final dynamic dashboard = tester.state(find.byType(MobileDashboard));
    expect(dashboard.settings()['username'], ' initial ');
    for (final username in [' user ', ' ']) {
      dashboard.open('上游代理设置');
      await tester.pumpAndSettle();
      await tester.enterText(find.byKey(const ValueKey('username')), username);
      await tester.tap(find.byTooltip('返回'));
      await tester.pumpAndSettle();
      expect(dashboard.page, '');
      expect(writes.last['username'], username);
      expect(dashboard.settings()['username'], username);
      await tester.tap(find.text('启动'));
      await tester.pumpAndSettle();
      expect(starts.last['username'], username);
      expect(starts.last['password'], ' secret ');
      await tester.tap(find.text('停止'));
      await tester.pumpAndSettle();
    }
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });
  testWidgets(
    'Home proxy toggle validates and keeps credentials; settings are grouped',
    (tester) async {
      tester.view.physicalSize = const Size(800, 1600);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final writes = <Map>[];
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(const MethodChannel('gbf/core'), (
            call,
          ) async {
            if (call.method == 'init') {
              return {
                'proxy': 'false',
                'host': '',
                'proxyPort': '7890',
                'username': 'user',
                'password': 'secret',
              };
            }
            if (call.method == 'settings') {
              writes.add(Map.of(call.arguments as Map));
            }
            return {'running': 'false'};
          });
      await tester.pumpWidget(const MaterialApp(home: MobileDashboard()));
      await tester.pumpAndSettle();
      final dynamic state = tester.state(find.byType(MobileDashboard));
      final upstream = find.widgetWithText(SwitchListTile, '上游代理');
      final connection = tester.widget<Card>(
        find.ancestor(of: upstream, matching: find.byType(Card)),
      );
      expect(
        tester.widget<Card>(
          find.ancestor(
            of: find.widgetWithText(SwitchListTile, '系统代理'),
            matching: find.byType(Card),
          ),
        ),
        same(connection),
      );
      expect(find.text('连接设置'), findsNothing);
      expect(find.text('缓存设置'), findsOneWidget);
      tester.widget<SwitchListTile>(upstream).onChanged!(true);
      await tester.pumpAndSettle();
      expect(state.page, '上游代理设置');
      expect(writes, isEmpty);
      expect(state.saved['proxy'], 'false');
      expect(find.byKey(const ValueKey('port')), findsNothing);
      state.fields['host'].text = '127.0.0.1';
      await state.back();
      await tester.pumpAndSettle();
      expect(writes.last['proxy'], 'true');
      expect(writes.last['password'], 'secret');
      tester.widget<SwitchListTile>(upstream).onChanged!(false);
      await tester.pumpAndSettle();
      expect(writes.last, {'proxy': 'false'});
      tester.widget<SwitchListTile>(upstream).onChanged!(true);
      await tester.pumpAndSettle();
      expect(state.page, '');
      expect(writes.last['password'], 'secret');
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets('Leaving an edited page waits for save before allowing start', (
    tester,
  ) async {
    final write = Completer<Map<String, String>>();
    var reads = 0;
    Map? start;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(const MethodChannel('gbf/core'), (
          call,
        ) async {
          if (call.method == 'settings') return write.future;
          if (call.method == 'status') reads++;
          if (call.method == 'start') start = call.arguments as Map;
          return {'running': 'false'};
        });
    await tester.pumpWidget(MaterialApp(home: const MobileDashboard()));
    await tester.pumpAndSettle();
    final dynamic state = tester.state(find.byType(MobileDashboard));
    state.open('缓存设置');
    await tester.pumpAndSettle();
    state.fields['memoryMiB'].text = '256';
    final saving = state.save() as Future<void>;
    await tester.pump();
    await tester.tap(find.byTooltip('返回'));
    await tester.pump();
    expect(find.text('缓存设置'), findsOneWidget);
    expect(start, isNull);
    final before = reads;
    write.complete({});
    await saving;
    await tester.pumpAndSettle();
    expect(reads, before);
    await tester.tap(find.text('启动'));
    await tester.pumpAndSettle();
    expect(start!['memoryMiB'], '256');
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('Mobile preference and cache saves omit unavailable passwords', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(390, 844);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final writes = <Map>[];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(const MethodChannel('gbf/core'), (
          call,
        ) async {
          if (call.method == 'init') {
            return {'passwordWarning': '密码恢复失败', 'password': ''};
          }
          if (call.method == 'settings') {
            writes.add(Map.of(call.arguments as Map));
          }
          return {'running': 'false'};
        });
    await tester.pumpWidget(
      MaterialApp(theme: ThemeData.dark(), home: const MobileDashboard()),
    );
    await tester.pumpAndSettle();
    final dynamic dashboard = tester.state(find.byType(MobileDashboard));
    dashboard.open('设置');
    await tester.pumpAndSettle();
    await tester.tap(find.text('保留日志'));
    await tester.pumpAndSettle();
    expect(find.text('启动后自动开启缓存服务'), findsNothing);
    dashboard.open('缓存设置');
    await tester.pumpAndSettle();
    await dashboard.save();
    expect(writes, [
      {'keepLogs': 'true'},
      {'memoryMiB': '128'},
    ]);
    expect(dashboard.settings().containsKey('password'), false);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets(
    'Mobile settings match desktop proxy controls and ignore retired settings',
    (tester) async {
      tester.view.physicalSize = const Size(390, 844);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final calls = <MethodCall>[];
      var running = false;
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(const MethodChannel('gbf/core'), (
            call,
          ) async {
            calls.add(call);
            if (call.method == 'init') {
              return {'capture': 'true', 'version': '0.9.9'};
            }
            if (call.method == 'start') running = true;
            if (call.method == 'stop') running = false;
            if (call.method == 'ca_status') {
              return {'state': 'valid', 'fingerprint': 'abc'};
            }
            if (call.method == 'ca') throw PlatformException(code: 'cancelled');
            return {
              'running': '$running',
              'preloaded': '42',
              'memoryHits': '17',
              'diskEntries': '12',
            };
          });
      await tester.pumpWidget(
        MaterialApp(theme: ThemeData.dark(), home: const MobileDashboard()),
      );
      await tester.pumpAndSettle();
      Future<void> tap(String text) async {
        if (find.byType(SnackBar).evaluate().isNotEmpty) {
          await tester.pump(const Duration(seconds: 4));
          await tester.pumpAndSettle();
        }
        final finder = find.text(text);
        if (finder.evaluate().isEmpty) {
          tester
              .state<ScrollableState>(find.byType(Scrollable).first)
              .position
              .jumpTo(0);
          await tester.pumpAndSettle();
          await tester.scrollUntilVisible(
            finder,
            200,
            scrollable: find.byType(Scrollable).first,
          );
        }
        await tester.ensureVisible(finder.last);
        await tester.pump(const Duration(milliseconds: 200));
        await tester.tap(finder.last);
        await tester.pump(const Duration(milliseconds: 300));
        if (find.byType(AlertDialog).evaluate().isEmpty) {
          await tester.pumpAndSettle();
        }
      }

      Future<void> back() async {
        await tester.tap(find.byTooltip('返回'));
        await tester.pumpAndSettle();
      }

      expect(find.text('42'), findsOneWidget);
      expect(find.text('网络异常'), findsOneWidget);
      await tap('上游代理');
      await tap('监听端口');
      expect(find.text('接管系统代理'), findsNothing);
      await tester.enterText(find.byKey(const ValueKey('port')), '9000');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      expect(calls.where((c) => c.method == 'settings'), isNotEmpty);
      await back();
      await tap('连通性检测');
      expect(
        (calls.lastWhere((c) => c.method == 'probe').arguments as Map)['port'],
        '9000',
      );
      await tap('上游代理设置');
      await tester.ensureVisible(find.byKey(const ValueKey('username')));
      await tester.enterText(find.byKey(const ValueKey('username')), 'user');
      await tester.ensureVisible(find.byKey(const ValueKey('password')));
      await tester.enterText(
        find.byKey(const ValueKey('password')),
        ' secret ',
      );
      await back();
      final saved =
          calls.lastWhere((c) => c.method == 'settings').arguments as Map;
      expect(saved['proxy'], 'true');
      expect(saved.containsKey('capture'), false);
      expect(saved.containsKey('connection'), false);
      expect(saved['password'], ' secret ');
      expect(saved.containsKey('memoryMiB'), false);
      await tap('CA 证书');
      expect(find.text('已生成'), findsOneWidget);
      await tap('导出 CA 证书');
      expect(find.text('CA 证书'), findsOneWidget);
      expect(
        find.text('PlatformException(cancelled, null, null, null)'),
        findsNothing,
      );
      await tap('重新生成证书');
      await tap('取消');
      expect(calls.where((c) => c.method == 'ca_regenerate'), isEmpty);
      await back();
      await tap('启动');
      final started =
          calls.lastWhere((c) => c.method == 'start').arguments as Map;
      expect(started['password'], ' secret ');
      expect(started.containsKey('capture'), false);
      expect(started.containsKey('connection'), false);
      expect(find.text('缓存服务运行中'), findsOneWidget);
      await tap('缓存设置');
      await tester.scrollUntilVisible(
        find.text('内存命中 17'),
        200,
        scrollable: find.byType(Scrollable).first,
      );
      expect(find.text('内存命中 17'), findsOneWidget);
      expect(
        tester
            .widget<TextField>(find.byKey(const ValueKey('memoryMiB')))
            .enabled,
        false,
      );
      await back();
      await tap('停止');
      final dynamic state = tester.state(find.byType(MobileDashboard));
      expect(state.running, false);
      await tap('上游代理设置');
      await tester.ensureVisible(find.byKey(const ValueKey('username')));
      await tester.enterText(find.byKey(const ValueKey('username')), '');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
      await tap('HTTP');
      await tap('SOCKS4');
      expect(find.byKey(const ValueKey('password')), findsNothing);
      await back();
      final socks4 =
          calls.lastWhere((c) => c.method == 'settings').arguments as Map;
      expect(socks4['protocol'], 'SOCKS4');
      expect(socks4['username'], '');

      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets(
    'LAN restart failure keeps saved config and refreshes immediately',
    (tester) async {
      tester.view.physicalSize = const Size(390, 844);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      final calls = <String>[];
      var running = true;
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(const MethodChannel('gbf/core'), (
            call,
          ) async {
            calls.add(call.method);
            if (call.method == 'init') return {'lan': 'false'};
            if (call.method == 'stop') running = false;
            if (call.method == 'start') {
              throw PlatformException(code: 'failed', message: '端口已被占用');
            }
            return {'running': '$running'};
          });
      await tester.pumpWidget(MaterialApp(home: const MobileDashboard()));
      await tester.pumpAndSettle();
      calls.clear();
      await tester.ensureVisible(find.text('局域网连接'));
      await tester.tap(find.text('局域网连接'));
      await tester.pump(const Duration(milliseconds: 300));
      await tester.tap(find.text('确认'));
      await tester.pumpAndSettle();
      expect(calls, ['stop', 'start', 'status']);
      final dynamic state = tester.state(find.byType(MobileDashboard));
      expect(state.running, false);
      expect(
        tester
            .widget<SwitchListTile>(
              find.widgetWithText(SwitchListTile, '局域网连接'),
            )
            .value,
        false,
      );
      await tester.scrollUntilVisible(
        find.text('端口已被占用'),
        -200,
        scrollable: find.byType(Scrollable).first,
      );
      expect(find.text('端口已被占用'), findsOneWidget);
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets('Memory capacity rejects overflow before saving', (tester) async {
    final writes = <Map>[];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(const MethodChannel('gbf/core'), (
          call,
        ) async {
          if (call.method == 'settings') writes.add(call.arguments as Map);
          return {'running': 'false'};
        });
    await tester.pumpWidget(MaterialApp(home: const MobileDashboard()));
    await tester.pumpAndSettle();
    await tester.scrollUntilVisible(
      find.text('缓存设置'),
      200,
      scrollable: find.byType(Scrollable).first,
    );
    await tester.ensureVisible(find.text('缓存设置'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('缓存设置'));
    await tester.pumpAndSettle();
    final field = find.byKey(const ValueKey('memoryMiB'));
    await tester.ensureVisible(field);
    await tester.enterText(field, '17592186044416');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(writes, isEmpty);
    await tester.ensureVisible(field);
    await tester.enterText(field, '8796093022207');
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pumpAndSettle();
    expect(writes.single['memoryMiB'], '8796093022207');
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('Android ignores old automatic-start preference', (tester) async {
    for (final sample in [
      (false, false, 0),
      (true, false, 0),
      (true, true, 0),
    ]) {
      final calls = <MethodCall>[];
      var running = sample.$2;
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(const MethodChannel('gbf/core'), (
            call,
          ) async {
            calls.add(call);
            if (call.method == 'init') return {'autoRun': '${sample.$1}'};
            if (call.method == 'start') running = true;
            return {'running': '$running'};
          });
      await tester.pumpWidget(
        MaterialApp(home: MobileDashboard(key: UniqueKey())),
      );
      await tester.pumpAndSettle();
      expect(calls.where((c) => c.method == 'start').length, sample.$3);
      expect(
        calls.any(
          (c) => (c.arguments as Map?)?.containsKey('capture') ?? false,
        ),
        false,
      );
      await tester.pumpWidget(const SizedBox());
    }
  });

  testWidgets('About retains own MIT license without dependency list', (
    tester,
  ) async {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('gbf/core'),
          (call) async => call.method == 'license'
              ? {'text': 'MIT License test text'}
              : {'running': 'false'},
        );
    await tester.pumpWidget(const MaterialApp(home: MobileDashboard()));
    await tester.pumpAndSettle();
    final dynamic state = tester.state(find.byType(MobileDashboard));
    state.open('设置');
    await tester.pumpAndSettle();
    await tester.ensureVisible(find.text('关于'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('关于'));
    await tester.pumpAndSettle();
    expect(find.byType(AboutDialog), findsNothing);
    expect(find.text('MIT 许可证'), findsOneWidget);
    await tester.tap(find.text('MIT 许可证'));
    await tester.pumpAndSettle();
    expect(find.text('MIT License test text'), findsOneWidget);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('Narrow screen with large text remains scrollable', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(360, 740);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('gbf/core'),
          (call) async => {'running': 'false'},
        );
    await tester.pumpWidget(
      MaterialApp(
        builder: (_, child) => MediaQuery(
          data: const MediaQueryData(textScaler: TextScaler.linear(1.5)),
          child: child!,
        ),
        home: const MobileDashboard(),
      ),
    );
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    await tester.ensureVisible(find.text('上游代理设置'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('上游代理设置'));
    await tester.pumpAndSettle();
    final dynamic state = tester.state(find.byType(MobileDashboard));
    expect(state.page, '上游代理设置');
    expect(find.widgetWithText(TextField, '127.0.0.1'), findsOneWidget);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });
  testWidgets('Invalid text cannot leave page or overwrite saved settings', (
    tester,
  ) async {
    final writes = <Map>[];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(const MethodChannel('gbf/core'), (
          call,
        ) async {
          if (call.method == 'init') return {'memoryMiB': '128'};
          if (call.method == 'settings') {
            writes.add(Map.of(call.arguments as Map));
          }
          return {'running': 'false'};
        });
    await tester.pumpWidget(const MaterialApp(home: MobileDashboard()));
    await tester.pumpAndSettle();
    final dynamic state = tester.state(find.byType(MobileDashboard));
    state.open('缓存设置');
    await tester.pumpAndSettle();
    state.fields['memoryMiB'].text = '-1';
    await tester.tap(find.byTooltip('返回'));
    await tester.pumpAndSettle();
    expect(state.page, '缓存设置');
    expect(writes, isEmpty);
    state.fields['memoryMiB'].text = '128';
    await tester.tap(find.byTooltip('返回'));
    await tester.pumpAndSettle();
    expect(state.page, '');
    expect(writes, isEmpty);
    await tester.pumpWidget(const SizedBox());
  });
  testWidgets('Theme choices open below the selector and save immediately', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(360, 740);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final writes = <Map>[];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(const MethodChannel('gbf/core'), (
          call,
        ) async {
          if (call.method == 'settings') {
            writes.add(Map.of(call.arguments as Map));
          }
          return {'running': 'false', 'themeMode': 'dark'};
        });
    await tester.pumpWidget(const MaterialApp(home: MobileDashboard()));
    await tester.pumpAndSettle();
    final dynamic dashboard = tester.state(find.byType(MobileDashboard));
    dashboard.open('设置');
    await tester.pumpAndSettle();
    for (final option in ['亮色', '跟随系统', '暗色']) {
      final selector = find.byKey(const ValueKey('theme-menu'));
      final bottom = tester.getRect(selector).bottom;
      await tester.tap(selector);
      await tester.pumpAndSettle();
      expect(
        tester.getRect(find.byType(PopupMenuItem<ThemeMode>).first).top,
        greaterThanOrEqualTo(bottom),
      );
      await tester.tap(find.widgetWithText(PopupMenuItem<ThemeMode>, option));
      await tester.pumpAndSettle();
      expect(
        find.descendant(of: selector, matching: find.text(option)),
        findsOneWidget,
      );
    }
    expect(writes.map((w) => w['themeMode']).toList(), [
      'light',
      'system',
      'dark',
    ]);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });
}
