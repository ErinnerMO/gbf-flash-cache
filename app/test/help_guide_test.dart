import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:gbf_flash_cache/mobile_dashboard.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:gbf_flash_cache/help_guide.dart';
import 'package:gbf_flash_cache/main.dart' show appTheme;

void main() {
  testWidgets('Mobile help survives scrolling, expansion and returning', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(390, 844);
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
        theme: appTheme(Brightness.dark),
        home: const MobileDashboard(),
      ),
    );
    await tester.pumpAndSettle();
    final dynamic dashboard = tester.state(find.byType(MobileDashboard));
    dashboard.open('使用说明');
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    await tester.drag(find.byType(ListView), const Offset(0, -400));
    await tester.pumpAndSettle();
    await tester.ensureVisible(find.text('连接检查与排查'));
    await tester.tap(find.text('连接检查与排查'));
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    await dashboard.back();
    await tester.pumpAndSettle();
    dashboard.open('使用说明');
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    await tester.ensureVisible(find.text('功能说明'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('功能说明'));
    await tester.pumpAndSettle();
    await tester.ensureVisible(find.text('缓存范围'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('缓存范围'));
    await tester.pumpAndSettle();
    expect(tester.takeException(), isNull);
    expect(find.byType(ErrorWidget), findsNothing);
    await tester.pumpWidget(const SizedBox());
  });
  for (final mobile in [false, true]) {
    testWidgets(
      '${mobile ? 'Android' : 'Windows'} help tabs and expanded sections',
      (tester) async {
        tester.view.physicalSize = mobile
            ? const Size(360, 740)
            : const Size(1120, 860);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.resetPhysicalSize);
        addTearDown(tester.view.resetDevicePixelRatio);
        await tester.pumpWidget(
          MaterialApp(
            theme: appTheme(Brightness.dark),
            home: Scaffold(
              body: MediaQuery(
                data: MediaQueryData(
                  textScaler: TextScaler.linear(mobile ? 1.4 : 1),
                ),
                child: SingleChildScrollView(child: HelpGuide(mobile: mobile)),
              ),
            ),
          ),
        );
        Future<void> open(String title) async {
          final tile = find.widgetWithText(ExpansionTile, title);
          await tester.ensureVisible(tile);
          await tester.pumpAndSettle();
          await tester.tap(
            find.descendant(of: tile, matching: find.text(title)),
          );
          await tester.pumpAndSettle();
          expect(tester.takeException(), isNull);
        }

        expect(find.text(mobile ? '浏览器接入' : '通过浏览器使用'), findsOneWidget);
        expect(find.text(verification), findsNothing);
        await open('连接检查与排查');
        expect(find.text(verification), findsOneWidget);
        if (mobile) {
          await open('系统代理 · 包含应用');
          await open('系统代理 · 排除应用');
        } else {
          expect(find.text('系统代理 · 包含应用'), findsNothing);
        }
        await tester.ensureVisible(find.text('功能说明'));
        await tester.tap(find.text('功能说明'));
        await tester.pumpAndSettle();
        expect(find.text('缓存范围'), findsOneWidget);
        await open('TCP／UDP 转发');
        expect(find.byType(Table), findsOneWidget);
        if (mobile) {
          await open('本地上游防回环');
          expect(find.text(localLoop), findsOneWidget);
          expect(find.text('Android 版本特性'), findsOneWidget);
          expect(find.text('Windows 版本特性'), findsNothing);
        } else {
          expect(find.text('Android 版本特性'), findsNothing);
          expect(find.text('Windows 版本特性'), findsOneWidget);
          await open('加速器兼容模式');
          expect(find.text(windowsAccelerator), findsOneWidget);
        }
        expect(tester.takeException(), isNull);
      },
    );
  }
}
