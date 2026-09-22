import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:gbf_flash_cache/mobile_dashboard.dart';

void main() {
  testWidgets(
    'Busy operations do not suppress stats; missing values are unknown',
    (tester) async {
      var calls = 0;
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(const MethodChannel('gbf/core'), (
            call,
          ) async {
            if (call.method == 'status') {
              calls++;
              return {'running': 'true', if (calls > 1) 'hits': '17'};
            }
            return <String, String>{};
          });
      await tester.pumpWidget(const MaterialApp(home: MobileDashboard()));
      await tester.pumpAndSettle();
      expect(find.text('—'), findsNWidgets(4));
      final dynamic state = tester.state(find.byType(MobileDashboard));
      final pending = Completer<void>();
      final operation = state.perform(
        () => pending.future,
        refreshStatus: false,
      ) as Future<void>;
      await tester.pump(const Duration(seconds: 3));
      await tester.pump();
      expect(calls, 2);
      expect(find.text('17'), findsOneWidget);
      pending.complete();
      await operation;
      await tester.pumpWidget(const SizedBox());
    },
  );
  testWidgets(
    'Initial status timeout recovers without queuing native requests',
    (tester) async {
      final stalled = Completer<Map<String, String>>();
      var calls = 0;
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(const MethodChannel('gbf/core'), (
            call,
          ) async {
            if (call.method == 'status') {
              calls++;
              if (calls == 1) return stalled.future;
              return {'running': 'true', 'hits': '29'};
            }
            return <String, String>{};
          });
      await tester.pumpWidget(const MaterialApp(home: MobileDashboard()));
      await tester.pump();
      await tester.pump();
      await tester.pump(const Duration(seconds: 11));
      await tester.pump();
      expect(find.text('统计刷新超时，等待服务响应'), findsOneWidget);
      await tester.pump(const Duration(seconds: 12));
      await tester.pump();
      expect(calls, 1);
      stalled.complete({'running': 'true', 'hits': '11'});
      await tester.pump();
      await tester.pump();
      await tester.pump(const Duration(seconds: 3));
      await tester.pump();
      expect(find.text('11'), findsOneWidget);
      expect(find.text('统计刷新超时，等待服务响应'), findsNothing);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await tester.pump();
      await tester.pump();
      expect(find.text('29'), findsOneWidget);
      expect(calls, 2);
      await tester.pumpWidget(const SizedBox());
    },
  );
}
