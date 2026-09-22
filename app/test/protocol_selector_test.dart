import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:gbf_flash_cache/protocol_selector.dart';

void main() {
  testWidgets('Protocol menu opens below the full field and selects once', (
    tester,
  ) async {
    String? selected;
    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: Align(
            alignment: Alignment.topCenter,
            child: SizedBox(
              width: 240,
              child: ProtocolSelector(
                value: 'SOCKS5',
                onChanged: (value) => selected = value,
              ),
            ),
          ),
        ),
      ),
    );
    final bottom = tester.getBottomLeft(find.byType(ProtocolSelector)).dy;
    await tester.tap(find.byType(ProtocolSelector));
    await tester.pumpAndSettle();
    final first = find.byWidgetPredicate(
      (w) => w is PopupMenuItem<String> && w.value == 'HTTP',
    );
    expect(tester.getTopLeft(first).dy, greaterThanOrEqualTo(bottom));
    await tester.tap(first);
    await tester.pumpAndSettle();
    expect(selected, 'HTTP');
    expect(find.byType(PopupMenuItem<String>), findsNothing);
  });
}
