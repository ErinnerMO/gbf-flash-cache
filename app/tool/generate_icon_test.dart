import 'dart:io';
import 'dart:typed_data';
import 'dart:ui';
import 'dart:ui' as ui;

import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets('Render shared lightning icon', (tester) async {
    await tester.runAsync(() async {
      final images = <int, List<int>>{};
      for (final size in [16, 24, 32, 48, 64, 128, 256, 512]) {
        final recorder = ui.PictureRecorder();
        final canvas = Canvas(recorder);
        canvas.scale(size / 100);
        final bounds = RRect.fromRectAndRadius(
          const Rect.fromLTWH(3, 3, 94, 94),
          const Radius.circular(25),
        );
        canvas.drawRRect(
          bounds,
          Paint()
            ..shader = ui.Gradient.linear(
              const Offset(5, 5),
              const Offset(90, 100),
              [const Color(0xFF254942), const Color(0xFF141D29)],
            ),
        );
        canvas.drawRRect(
          bounds,
          Paint()
            ..color = const Color(0xFF55AB9A)
            ..style = PaintingStyle.stroke
            ..strokeWidth = 2,
        );
        final bolt = Path()
          ..moveTo(57, 20)
          ..lineTo(31, 55)
          ..lineTo(46, 55)
          ..lineTo(40, 81)
          ..lineTo(70, 43)
          ..lineTo(54, 43)
          ..close();
        canvas.drawPath(bolt, Paint()..color = const Color(0xFF85DFCB));
        final image = await recorder.endRecording().toImage(size, size);
        images[size] = (await image.toByteData(format: ui.ImageByteFormat.png))!
            .buffer
            .asUint8List();
        image.dispose();
      }
      await File('assets/app-icon.png').writeAsBytes(images[512]!);
      await File('android/app/src/main/res/drawable-nodpi/app_icon.png')
          .writeAsBytes(images[512]!);
      final sizes = images.keys.where((s) => s <= 256).toList();
      final header = ByteData(6 + sizes.length * 16)
        ..setUint16(2, 1, Endian.little)
        ..setUint16(4, sizes.length, Endian.little);
      var offset = header.lengthInBytes;
      for (var i = 0; i < sizes.length; i++) {
        final size = sizes[i], at = 6 + i * 16;
        header.setUint8(at, size == 256 ? 0 : size);
        header.setUint8(at + 1, size == 256 ? 0 : size);
        header.setUint16(at + 4, 1, Endian.little);
        header.setUint16(at + 6, 32, Endian.little);
        header.setUint32(at + 8, images[size]!.length, Endian.little);
        header.setUint32(at + 12, offset, Endian.little);
        offset += images[size]!.length;
      }
      final ico = [
        ...header.buffer.asUint8List(),
        for (final size in sizes) ...images[size]!,
      ];
      await File('assets/app-icon.ico').writeAsBytes(ico);
      await File('windows/runner/resources/app_icon.ico').writeAsBytes(ico);
    });
  });
}
