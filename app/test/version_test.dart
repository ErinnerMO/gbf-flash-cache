import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:gbf_flash_cache/main.dart' show appVersion;

import '../tool/check_version.dart';

void main() {
  test('Desktop default version matches the current release', () {
    checkVersion(Directory('..'), appVersion);
  });
  test('Release version must match both Rust crates and Flutter', () {
    final root = Directory.systemTemp.createTempSync('gbf-version-');
    addTearDown(() => root.deleteSync(recursive: true));
    for (final path in ['core/Cargo.toml', 'app/native/Cargo.toml']) {
      final file = File('${root.path}/$path');
      file.parent.createSync(recursive: true);
      file.writeAsStringSync(
        '[package]\nname = "test"\nversion = "0.9.9"\n[dependencies]\n',
      );
    }
    File('${root.path}/app/pubspec.yaml')
        .writeAsStringSync('version: 0.9.9+1\n');
    expect(() => checkVersion(root, '0.9.9'), returnsNormally);
    expect(() => checkVersion(root, '0.9.10'), throwsStateError);
    File('${root.path}/app/native/Cargo.toml')
        .writeAsStringSync('[package]\nversion = "0.9.8"\n');
    expect(() => checkVersion(root, '0.9.9'), throwsStateError);
    File('${root.path}/app/native/Cargo.toml')
        .writeAsStringSync('[package]\nversion = "0.9.9"\n');
    File('${root.path}/app/pubspec.yaml')
        .writeAsStringSync('version: 0.9.10+1\n');
    expect(() => checkVersion(root, '0.9.9'), throwsStateError);
  });
}
