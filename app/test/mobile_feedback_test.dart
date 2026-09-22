import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:gbf_flash_cache/mobile_dashboard.dart';

void main() {
  testWidgets('Home keeps scroll position across settings pages', (
    tester,
  ) async {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('gbf/core'),
          (_) async => {'running': 'false'},
        );
    await tester.pumpWidget(const MaterialApp(home: MobileDashboard()));
    await tester.pumpAndSettle();
    await tester.scrollUntilVisible(
      find.text('缓存设置'),
      200,
      scrollable: find.byType(Scrollable).first,
    );
    await tester.ensureVisible(find.text('缓存设置'));
    await tester.pumpAndSettle();
    final before = tester
        .state<ScrollableState>(find.byType(Scrollable).first)
        .position
        .pixels;
    expect(before, greaterThan(0));
    await tester.tap(find.text('缓存设置'));
    await tester.pumpAndSettle();
    await tester.tap(find.byTooltip('返回'));
    await tester.pumpAndSettle();
    expect(
      tester
          .state<ScrollableState>(find.byType(Scrollable).first)
          .position
          .pixels,
      closeTo(before, 1),
    );
    await tester.pumpWidget(const SizedBox());
  });
  testWidgets(
    'Probe shows inline pending success and failure, edits clear it',
    (tester) async {
      var probe = Completer<Map<String, String>>();
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(const MethodChannel('gbf/core'), (
            call,
          ) async {
            if (call.method == 'probe') return probe.future;
            return {'running': 'false'};
          });
      await tester.pumpWidget(const MaterialApp(home: MobileDashboard()));
      await tester.pumpAndSettle();
      final dynamic state = tester.state(find.byType(MobileDashboard));
      await tester.scrollUntilVisible(
        find.text('连通性检测'),
        200,
        scrollable: find.byType(Scrollable).first,
      );
      await tester.ensureVisible(find.text('连通性检测'));
      await tester.tap(find.text('连通性检测'));
      await tester.pump();
      expect(find.byType(CircularProgressIndicator), findsOneWidget);
      expect(
        find.byWidgetPredicate(
          (widget) => widget is LinearProgressIndicator && widget.value == null,
        ),
        findsNothing,
      );
      probe.complete({});
      await tester.pumpAndSettle();
      expect(find.text('连接正常'), findsOneWidget);
      expect(find.byType(SnackBar), findsNothing);
      probe = Completer<Map<String, String>>();
      await tester.ensureVisible(find.text('连通性检测'));
      await tester.tap(find.text('连通性检测'));
      await tester.pump();
      probe.completeError(PlatformException(code: 'probe', message: '连接超时'));
      await tester.pumpAndSettle();
      expect(find.text('连接失败'), findsOneWidget);
      expect(find.text('连接超时'), findsOneWidget);
      expect(find.byType(SnackBar), findsNothing);
      state.open('上游代理设置');
      await tester.pumpAndSettle();
      expect(find.text('检测连接'), findsNothing);
      await tester.ensureVisible(find.byKey(const ValueKey('host')));
      await tester.enterText(find.byKey(const ValueKey('host')), 'localhost');
      await tester.pump();
      expect(find.text('连接失败'), findsNothing);
      await tester.pumpWidget(const SizedBox());
    },
  );
}
