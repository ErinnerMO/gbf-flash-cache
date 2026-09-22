import 'dart:async';

import 'package:flutter/material.dart';
import 'package:window_manager/window_manager.dart';
import 'package:tray_manager/tray_manager.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:gbf_flash_cache/main.dart';
import 'package:gbf_flash_cache/core_client.dart';

class FakeCore extends CoreClient {
  FakeCore({this.initFields = const {}, this.directoryWarning = ''});
  String directoryWarning;
  final Map<String, String> initFields;
  bool running = false, failSave = false;
  Completer<void>? saveGate;
  final starts = <Map<String, String>>[];
  Completer<void>? startGate, closeGate;
  @override
  Future<void> close() async {
    await closeGate?.future;
  }

  final writes = <Map<String, String>>[];
  final probes = <Map<String, String>>[];
  @override
  Future<void> connect() async {}
  @override
  Future<Map<String, String>> call(
    String op, [
    Map<String, String> args = const {},
  ]) async {
    if (op == 'settings') {
      writes.add(Map.of(args));
      await saveGate?.future;
      if (failSave) throw StateError('settings write failed');
    }
    if (op == 'probe') probes.add(Map.of(args));
    if (op == 'start') {
      starts.add(Map.of(args));
      await startGate?.future;
      running = true;
    }
    if (op == 'stop') running = false;
    return {
      'state': 'valid',
      'trusted': 'false',
      'fingerprint': 'test-certificate',
      'home': '/tmp/test-data',
      'directoryWarning': directoryWarning,
      'running': '$running',
      'hits': '24',
      'requests': '8',
      'diskBytes': '1048576',
      if (op == 'init') ...initFields,
    };
  }
}

void main() {
  testWidgets(
    'Compatibility setting defaults off, persists and only changes after save succeeds',
    (tester) async {
      tester.view.physicalSize = const Size(1120, 1100);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      for (final name in ['window_manager', 'tray_manager']) {
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
            .setMockMethodCallHandler(
              MethodChannel(name),
              (call) async => call.method.startsWith('is') ? false : null,
            );
      }
      final core = FakeCore();
      await tester.pumpWidget(FlashApp(client: core));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('设置'));
      await tester.pumpAndSettle();
      final toggle = find.widgetWithText(SwitchListTile, '加速器兼容模式');
      expect(tester.widget<SwitchListTile>(toggle).value, isFalse);
      expect(
        find.text('以 chrome.exe 进程名运行，使按进程名识别的游戏加速器能够接管流量。切换需要重启应用。'),
        findsOneWidget,
      );
      expect(find.text('立即重启'), findsNothing);
      await tester.ensureVisible(toggle);
      await tester.tap(toggle);
      await tester.pumpAndSettle();
      expect(core.writes.last, {'acceleratorCompatibility': 'true'});
      expect(tester.widget<SwitchListTile>(toggle).value, isTrue);
      expect(find.text('立即重启'), findsOneWidget);
      core.failSave = true;
      await tester.tap(toggle);
      await tester.pumpAndSettle();
      expect(tester.widget<SwitchListTile>(toggle).value, isTrue);
      core.failSave = false;
      await tester.ensureVisible(toggle);
      await tester.tap(toggle);
      await tester.pumpAndSettle();
      expect(core.writes.last, {'acceleratorCompatibility': 'false'});
      expect(find.text('立即重启'), findsNothing);
      await tester.pumpWidget(const SizedBox());
      await tester.pumpWidget(
        FlashApp(
          client: FakeCore(initFields: {'acceleratorCompatibility': 'true'}),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('设置'));
      await tester.pumpAndSettle();
      expect(tester.widget<SwitchListTile>(toggle).value, isTrue);
      expect(find.text('立即重启'), findsOneWidget);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
    },
  );
  test(
    'Switch off state is visible without overriding selected or disabled',
    () {
      for (final brightness in Brightness.values) {
        final theme = appTheme(brightness);
        for (final color in [
          theme.switchTheme.thumbColor,
          theme.switchTheme.trackOutlineColor,
        ]) {
          expect(color!.resolve({}), theme.textTheme.bodySmall!.color);
          expect(color.resolve({WidgetState.selected}), isNull);
          expect(color.resolve({WidgetState.disabled}), isNull);
          expect(
            color.resolve({WidgetState.disabled, WidgetState.selected}),
            isNull,
          );
        }
      }
    },
  );
  testWidgets('Desktop theme, field autosave and input validation', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1120, 860);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    for (final name in ['window_manager', 'tray_manager']) {
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(
            MethodChannel(name),
            (call) async => call.method.startsWith('is') ? false : null,
          );
    }
    final core = FakeCore(initFields: {'themeMode': 'light'});
    await tester.pumpWidget(FlashApp(client: core));
    await tester.pumpAndSettle();
    expect(
      Theme.of(tester.element(find.byType(Dashboard))).brightness,
      Brightness.light,
    );
    expect(
      tester.getTopLeft(find.text('局域网连接')).dy,
      lessThan(tester.getTopLeft(find.text('上游代理')).dy),
    );
    await tester.tap(find.byTooltip('设置'));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('theme-menu')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('暗色').last);
    await tester.pumpAndSettle();
    expect(core.writes.last, {'themeMode': 'dark'});
    expect(
      Theme.of(tester.element(find.byType(Dashboard))).brightness,
      Brightness.dark,
    );
    core.failSave = true;
    await tester.tap(find.byKey(const ValueKey('theme-menu')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('亮色').last);
    await tester.pumpAndSettle();
    expect(
      Theme.of(tester.element(find.byType(Dashboard))).brightness,
      Brightness.dark,
    );
    core.failSave = false;
    await tester.tap(find.byTooltip('服务'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('连接设置'));
    await tester.pumpAndSettle();
    expect(find.text('保存'), findsNothing);
    expect(
      find.descendant(
        of: find.byType(AlertDialog),
        matching: find.byType(Switch),
      ),
      findsNothing,
    );
    await tester.enterText(find.byType(TextField).first, '9999');
    FocusManager.instance.primaryFocus?.unfocus();
    await tester.pumpAndSettle();
    expect(core.writes.last['port'], '9999');
    expect(find.byType(AlertDialog), findsOneWidget);
    final count = core.writes.length;
    await tester.enterText(find.byType(TextField).first, '0');
    await tester.tap(find.byTooltip('关闭设置'));
    await tester.pumpAndSettle();
    expect(core.writes.length, count);
    expect(find.text('请填写有效地址和 1–65535 的端口'), findsOneWidget);
    await tester.enterText(find.byType(TextField).first, '9998');
    await tester.binding.handlePopRoute();
    await tester.pumpAndSettle();
    expect(core.writes.last['port'], '9998');
    expect(find.byType(AlertDialog), findsNothing);
    expect(find.byType(Dashboard), findsOneWidget);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });
  for (final setting in ['keepLogs', 'port', 'proxy']) {
    final connection = setting == 'port';
    testWidgets('Save failure preserves $setting settings', (tester) async {
      tester.view.physicalSize = const Size(1120, 860);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.resetPhysicalSize);
      addTearDown(tester.view.resetDevicePixelRatio);
      for (final name in ['window_manager', 'tray_manager']) {
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
            .setMockMethodCallHandler(
              MethodChannel(name),
              (call) async => call.method.startsWith('is') ? false : null,
            );
      }
      final core = FakeCore()..failSave = true;
      await tester.pumpWidget(FlashApp(client: core));
      await tester.pumpAndSettle();
      final dynamic dashboard = tester.state(find.byType(Dashboard));
      final logSwitch = find.descendant(
        of: find
            .ancestor(
              of: find.text(setting == 'proxy' ? '上游代理' : '保留日志'),
              matching: find.byType(Row),
            )
            .first,
        matching: find.byType(Switch),
      );
      Future<void> save() async {
        if (connection) {
          await tester.tap(find.byTooltip('连接设置'));
          await tester.pumpAndSettle();
          await tester.enterText(find.byType(TextField).first, '9999');
          await tester.tap(find.byTooltip('关闭设置'));
        } else {
          await tester.tap(logSwitch);
        }
      }

      await save();
      await tester.pumpAndSettle();
      expect(find.text('settings write failed'), findsOneWidget);
      expect(dashboard.settings()['port'], '8765');
      expect(dashboard.settings()['keepLogs'], 'false');
      expect(dashboard.settings()['proxy'], 'false');
      if (connection) {
        expect(find.byType(AlertDialog), findsOneWidget);
        expect(find.text('9999'), findsOneWidget);
        Navigator.of(tester.element(find.byType(AlertDialog))).pop();
        await tester.pumpAndSettle();
      } else {
        expect(tester.widget<Switch>(logSwitch).value, false);
      }
      await tester.tap(find.text('启动服务'));
      await tester.pumpAndSettle();
      expect(core.starts.last['port'], '8765');
      expect(core.starts.last['keepLogs'], 'false');
      expect(core.starts.last['proxy'], 'false');
      await tester.tap(find.text('停止服务'));
      await tester.pumpAndSettle();
      core.failSave = false;
      core.saveGate = Completer<void>();
      await save();
      await tester.pump();
      expect(dashboard.settings()['port'], '8765');
      expect(dashboard.settings()['keepLogs'], 'false');
      expect(dashboard.settings()['proxy'], 'false');
      core.saveGate!.complete();
      await tester.pumpAndSettle();
      expect(dashboard.settings()[setting], connection ? '9999' : 'true');
      expect(find.byType(AlertDialog), findsNothing);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
    });
  }

  testWidgets('Cache save blocks dismissal and conflicting service start', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1120, 860);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    for (final name in ['window_manager', 'tray_manager']) {
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(
            MethodChannel(name),
            (call) async => call.method.startsWith('is') ? false : null,
          );
    }
    final core = FakeCore()..saveGate = Completer<void>();
    await tester.pumpWidget(FlashApp(client: core));
    await tester.pumpAndSettle();
    final dynamic dashboard = tester.state(find.byType(Dashboard));
    unawaited(dashboard.cacheSettings() as Future<void>);
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField).last, '256');
    await tester.tap(find.byTooltip('关闭设置'));
    await tester.pump();
    await tester.tapAt(const Offset(5, 5));
    await tester.pump();
    await tester.binding.handlePopRoute();
    await tester.pump();
    expect(find.byType(AlertDialog), findsOneWidget);
    expect(dashboard.busy, true);
    await dashboard.toggle();
    expect(core.starts, isEmpty);
    expect(dashboard.settings()['memoryMiB'], '128');
    core.saveGate!.complete();
    await tester.pumpAndSettle();
    expect(find.byType(AlertDialog), findsNothing);
    expect(dashboard.busy, false);
    await tester.tap(find.text('启动服务'));
    await tester.pumpAndSettle();
    expect(core.starts.single['memoryMiB'], '256');
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('Unavailable directories retain both repair dialogs', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1120, 860);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    for (final name in ['window_manager', 'tray_manager']) {
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(
            MethodChannel(name),
            (call) async => call.method.startsWith('is') ? false : null,
          );
    }
    final core = FakeCore(
      directoryWarning: '缓存、日志目录不可用',
      initFields: {'autoRun': 'true'},
    );
    await tester.pumpWidget(FlashApp(client: core));
    await tester.pumpAndSettle();
    final dynamic dashboard = tester.state(find.byType(Dashboard));
    expect(dashboard.ready, true);
    expect(core.starts, isEmpty);
    expect(find.text('缓存、日志目录不可用'), findsOneWidget);
    await tester.tap(find.byTooltip('缓存设置'));
    await tester.pumpAndSettle();
    expect(find.widgetWithText(TextButton, '选择目录'), findsOneWidget);
    await tester.tap(find.byTooltip('关闭设置'));
    await tester.pumpAndSettle();
    await tester.ensureVisible(find.widgetWithText(TextButton, '日志目录'));
    await tester.pumpAndSettle();
    await tester.tap(find.widgetWithText(TextButton, '日志目录'));
    await tester.pumpAndSettle();
    expect(find.byType(AlertDialog), findsOneWidget);
    await tester.tap(find.text('取消'));
    await tester.pumpAndSettle();
    core.directoryWarning = '';
    await dashboard.refresh();
    await tester.pumpAndSettle();
    expect(find.text('缓存、日志目录不可用'), findsNothing);
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('Preferences do not submit unavailable passwords', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1120, 860);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    for (final name in ['window_manager', 'tray_manager']) {
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(
            MethodChannel(name),
            (call) async => call.method.startsWith('is') ? false : null,
          );
    }
    final core = FakeCore(
      initFields: {'passwordWarning': '密码恢复失败', 'password': ''},
    );
    await tester.pumpWidget(FlashApp(client: core));
    await tester.pumpAndSettle();
    final dynamic dashboard = tester.state(find.byType(Dashboard));
    await dashboard.setPreference('keepLogs', true);
    await dashboard.setLan(true);
    expect(core.writes, [
      {'keepLogs': 'true'},
      {'lan': 'true'},
    ]);
    expect(dashboard.settings().containsKey('password'), false);
    await tester.pumpWidget(const SizedBox());
  });

  test('Windows display paths preserve drive and UNC paths', () {
    expect(displayPath(r'\\?\C:\cache'), r'C:\cache');
    expect(displayPath(r'\\?\UNC\server\cache'), r'\\server\cache');
    expect(displayPath('/tmp/cache'), '/tmp/cache');
  });
  testWidgets('Unreadable password allows settings without auto start', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1120, 860);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    for (final name in ['window_manager', 'tray_manager']) {
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(
            MethodChannel(name),
            (call) async => call.method.startsWith('is') ? false : null,
          );
    }
    final core = FakeCore(
      initFields: {
        'passwordWarning': '上游代理密码无法解密，请重新填写并保存',
        'password': '',
        'autoRun': 'true',
        'proxy': 'false',
        'protocol': 'HTTP',
        'username': 'user',
      },
    );
    await tester.pumpWidget(FlashApp(client: core));
    await tester.pumpAndSettle();
    expect(core.running, false);
    expect(find.text('上游代理密码无法解密，请重新填写并保存'), findsOneWidget);
    await tester.tap(find.byTooltip('连接设置'));
    await tester.pumpAndSettle();
    expect(find.byTooltip('关闭设置'), findsOneWidget);
    final fields = find.byType(TextFormField);
    for (final field in tester.widgetList<TextFormField>(fields)) {
      expect(field.enabled, isNot(false));
    }
    await tester.enterText(fields.last, 'replacement-secret');
    await tester.tap(find.byTooltip('关闭设置'));
    await tester.pumpAndSettle();
    expect(core.writes.last['password'], 'replacement-secret');
    expect(core.writes.last['proxy'], 'false');
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });
  testWidgets('Desktop saves and starts with exact proxy usernames', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1120, 860);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    for (final name in ['window_manager', 'tray_manager']) {
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(
            MethodChannel(name),
            (call) async => call.method.startsWith('is') ? false : null,
          );
    }
    final core = FakeCore(
      initFields: {
        'proxy': 'true',
        'host': '127.0.0.1',
        'proxyPort': '1080',
        'username': ' initial ',
        'password': ' secret ',
      },
    );
    await tester.pumpWidget(FlashApp(client: core));
    await tester.pumpAndSettle();
    final dynamic dashboard = tester.state(find.byType(Dashboard));
    expect(dashboard.settings()['username'], ' initial ');
    for (final username in [' user ', ' ']) {
      await tester.tap(find.byTooltip('连接设置'));
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextFormField).at(3), username);
      await tester.tap(find.byTooltip('关闭设置'));
      await tester.pumpAndSettle();
      expect(find.byType(AlertDialog), findsNothing);
      expect(core.writes.last['username'], username);
      expect(dashboard.settings()['username'], username);
      await tester.tap(find.text('启动服务'));
      await tester.pumpAndSettle();
      expect(core.starts.last['username'], username);
      expect(core.starts.last['password'], ' secret ');
      await tester.tap(find.text('停止服务'));
      await tester.pumpAndSettle();
    }
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox());
  });
  testWidgets('Saving password-only proxy credentials matches Android', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1120, 860);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    for (final name in ['window_manager', 'tray_manager']) {
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(
            MethodChannel(name),
            (call) async => call.method.startsWith('is') ? false : null,
          );
    }
    for (final protocol in ['HTTP', 'HTTPS', 'SOCKS5', 'SOCKS4']) {
      final core = FakeCore(
        initFields: {
          'proxy': 'true',
          'protocol': protocol,
          'host': '127.0.0.1',
          'proxyPort': '1080',
          'username': '',
          'password': 'secret',
        },
      );
      await tester.pumpWidget(FlashApp(client: core));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('连接设置'));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('关闭设置'));
      await tester.pumpAndSettle();
      if (protocol == 'SOCKS4') {
        expect(core.writes, isEmpty);
      } else {
        expect(core.writes, isEmpty);
        expect(find.byType(AlertDialog), findsOneWidget);
        expect(find.text('请填写上游代理用户名'), findsOneWidget);
      }
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox());
    }
  });
  testWidgets('Dashboard controls the service and hides About behind an icon', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(1120, 860);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final windowCalls = <String>[];
    for (final name in ['window_manager', 'tray_manager']) {
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(MethodChannel(name), (call) async {
            if (name == 'window_manager') windowCalls.add(call.method);
            return call.method.startsWith('is') ? false : null;
          });
    }
    final core = FakeCore();
    await tester.pumpWidget(FlashApp(client: core));
    await tester.pumpAndSettle();
    expect(find.text('未启动'), findsOneWidget);
    expect(find.text('24'), findsOneWidget);
    expect(tester.getBottomRight(find.text('导出日志')).dy, lessThan(860));
    await tester.tap(find.text('启动服务'));
    await tester.pumpAndSettle();
    expect(find.text('运行中'), findsOneWidget);
    await tester.tap(find.text('停止服务'));
    await tester.pumpAndSettle();
    expect(find.text('未启动'), findsOneWidget);
    tester.view.physicalSize = const Size(1120, 720);
    await tester.tap(find.byTooltip('连接设置'));
    await tester.pumpAndSettle();
    expect(
      tester.getBottomRight(find.byType(TextField).last).dy,
      lessThan(tester.getBottomRight(find.byType(AlertDialog)).dy),
    );
    await tester.enterText(find.byType(TextField).first, '9999');
    await tester.ensureVisible(find.text('检测连接'));
    await tester.tap(find.text('检测连接'));
    await tester.pumpAndSettle();
    expect(core.probes.single['port'], '9999');
    expect(core.writes.last['port'], '9999');
    expect(find.text('当前配置可以访问游戏站点'), findsOneWidget);
    await tester.tap(find.byTooltip('关闭设置'));
    await tester.pumpAndSettle();
    expect(find.text('监听端口 9999'), findsOneWidget);
    expect(tester.takeException(), isNull);
    tester.view.physicalSize = const Size(1120, 860);
    await tester.tap(find.byTooltip('缓存设置'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextFormField), '256');
    await tester.tap(find.byTooltip('关闭设置'));
    await tester.pumpAndSettle();
    expect(core.writes.last['memoryMiB'], '256');
    await tester.tap(find.byTooltip('缓存设置'));
    await tester.pumpAndSettle();
    expect(find.text('256'), findsOneWidget);
    await tester.tap(find.byTooltip('关闭设置'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('设置'));
    await tester.pumpAndSettle();
    expect(
      tester
          .widget<SwitchListTile>(
            find.widgetWithText(SwitchListTile, '关闭窗口时隐藏到托盘'),
          )
          .value,
      isTrue,
    );
    expect(
      tester
          .widget<SwitchListTile>(
            find.widgetWithText(SwitchListTile, '启动后自动开启缓存服务'),
          )
          .value,
      isFalse,
    );
    await tester.tap(find.text('启动后自动开启缓存服务'));
    await tester.pumpAndSettle();
    expect(core.writes.last['autoRun'], 'true');
    await tester.tap(find.byTooltip('服务'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('管理证书'));
    await tester.pumpAndSettle();
    expect(find.text('Windows 系统信任'), findsOneWidget);
    expect(find.text('安装证书'), findsOneWidget);
    expect(tester.takeException(), isNull);
    await tester.tap(find.text('完成'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('关于'));
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    await tester.tap(find.text('关闭'));
    await tester.pumpAndSettle();
    core.startGate = Completer<void>();
    core.closeGate = Completer<void>();
    await tester.tap(find.text('启动服务'));
    await tester.pump();
    expect(find.text('请稍候'), findsOneWidget);
    (tester.state(find.byType(Dashboard)) as WindowListener).onWindowClose();
    await tester.pump();
    expect(windowCalls, contains('hide'));
    expect(windowCalls, isNot(contains('destroy')));
    expect(core.closeGate!.isCompleted, isFalse);
    (tester.state(find.byType(Dashboard)) as TrayListener).onTrayMenuItemClick(
      MenuItem(key: 'exit', label: '退出'),
    );
    await tester.pump();
    core.startGate!.complete();
    core.closeGate!.complete();
    await tester.pumpAndSettle();
    expect(windowCalls, contains('destroy'));
    await tester.pumpWidget(const SizedBox());
  });
}
