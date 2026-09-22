import 'dart:convert';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:gbf_flash_cache/platform/windows.dart';
import 'package:path/path.dart' as p;

void main() {
  test('Compatibility copy preserves the original and data, refreshes and switches back', () async {
    final dir = await Directory.systemTemp.createTemp('gfc launcher 中文 ');
    addTearDown(() => dir.delete(recursive: true));
    final original = File(p.join(dir.path, 'GBF Flash Cache.exe'));
    await original.writeAsString('version 1');
    final launcher = WindowsLauncher(original.path);
    expect(await launcher.enabled(), isFalse);
    expect(await launcher.prepare(false), isNull);
    final copy = (await launcher.prepare(true))!;
    expect(p.basename(copy.path), 'chrome.exe');
    expect(await copy.readAsString(), 'version 1');
    expect(await original.readAsString(), 'version 1');
    final previousModified = DateTime(2020);
    await copy.setLastModified(previousModified);
    await launcher.prepare(true);
    expect(await copy.lastModified(), previousModified);
    final settings = File(p.join(dir.path, 'data', 'settings.json'));
    await settings.writeAsString(
      '{"acceleratorCompatibility":"true","port":"9999"}',
    );
    final compatible = WindowsLauncher(copy.path);
    expect(await compatible.enabled(), isTrue);
    expect(await compatible.prepare(true), isNull); // No relaunch loop.
    expect((await compatible.prepare(false))!.path, original.path);
    await original.writeAsString('version 2');
    await launcher.prepare(true);
    expect(await copy.readAsString(), 'version 2');
    expect(await settings.readAsString(), contains('"port":"9999"'));
    await original.delete();
    await expectLater(
      launcher.prepare(true),
      throwsA(isA<FileSystemException>()),
    );
    expect(await copy.readAsString(), 'version 2');
    await expectLater(compatible.prepare(false), throwsStateError);
    await compatible.sourceRecord.writeAsString('../outside.exe');
    await expectLater(compatible.prepare(false), throwsStateError);
  });

  test(
    'Windows reuses a locked copy and waits for a changed copy to unlock',
    () async {
      final dir = await Directory.systemTemp.createTemp('gfc locked launcher ');
      addTearDown(() => dir.delete(recursive: true));
      final original = File(p.join(dir.path, 'GBF Flash Cache.exe'));
      await original.writeAsString('version 1');
      final launcher = WindowsLauncher(original.path);
      final copy = (await launcher.prepare(true))!;
      final script = File(p.join(dir.path, 'lock.ps1'));
      await script.writeAsString(r'''
$file = [IO.File]::Open((Join-Path $PSScriptRoot 'chrome.exe'), 'Open', 'Read', 'Read')
try { [Console]::WriteLine('locked'); Start-Sleep -Seconds 120 }
finally { $file.Dispose() }
''');
      final holder = await Process.start('powershell.exe', [
        '-NoProfile',
        '-ExecutionPolicy',
        'Bypass',
        '-File',
        script.path,
      ]);
      addTearDown(() async {
        holder.kill();
        await holder.exitCode;
      });
      expect(
        await holder.stdout
            .transform(utf8.decoder)
            .transform(const LineSplitter())
            .first,
        'locked',
      );
      expect((await launcher.prepare(true))!.path, copy.path);
      await original.writeAsString('version 2');
      var completed = false;
      final update = launcher.prepare(true).then((value) {
        completed = true;
        return value;
      });
      await Future<void>.delayed(const Duration(milliseconds: 300));
      expect(completed, isFalse);
      expect(await copy.readAsString(), 'version 1');
      holder.kill();
      await holder.exitCode;
      await update;
      expect(await copy.readAsString(), 'version 2');
    },
    skip: !Platform.isWindows,
  );

  test('Unowned chrome.exe and invalid settings fail without changing existing files', () async {
    final dir = await Directory.systemTemp.createTemp('gfc-launcher-');
    addTearDown(() => dir.delete(recursive: true));
    final original = File(p.join(dir.path, 'GBF Flash Cache.exe'));
    await original.writeAsString('gfc');
    final chrome = File(p.join(dir.path, 'chrome.exe'));
    await chrome.writeAsString('unrelated browser');
    final launcher = WindowsLauncher(original.path);
    await expectLater(launcher.prepare(true), throwsStateError);
    expect(await chrome.readAsString(), 'unrelated browser');
    await Directory(p.join(dir.path, 'data')).create();
    await File(p.join(dir.path, 'data', 'settings.json')).writeAsString('{');
    await expectLater(launcher.enabled(), throwsFormatException);
  });
}
