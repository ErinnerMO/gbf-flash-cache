import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart' show listEquals;
import 'package:path/path.dart' as p;

import '../core_client.dart';

/// Windows application paths and shell integration; no cache policies.
class WindowsHost {
  static String get directory => p.dirname(Platform.resolvedExecutable);
  static final launcher = WindowsLauncher(Platform.resolvedExecutable);
  static String startupWarning = '';
  static CoreClient createClient() {
    if (!Platform.isWindows) throw UnsupportedError('桌面版目前仅支持 Windows');
    return CoreClient(
      libraryPath: p.join(directory, 'core', 'gbf_flash_cache_app.dll'),
      dataDirectory: p.join(directory, 'data'),
    );
  }

  static String get trayIcon => p.join(
    directory,
    'ui-assets',
    'flutter_assets',
    'assets',
    'app-icon.ico',
  );
  static String get licensePath => p.join(directory, 'LICENSE');
  static Future<void> reveal(String path) async {
    await Process.run('explorer.exe', [path]);
  }
}

/// The original EXE remains the entry point; both names share the same data.
class WindowsLauncher {
  WindowsLauncher(this.executable);
  final String executable;
  String get directory => p.dirname(executable);
  bool get compatible => p.basename(executable).toLowerCase() == 'chrome.exe';
  File get sourceRecord =>
      File(p.join(directory, 'data', 'windows-launcher.txt'));

  Future<bool> enabled() async {
    final settings = File(p.join(directory, 'data', 'settings.json'));
    if (!await settings.exists()) return false;
    return (jsonDecode(await settings.readAsString())
            as Map)['acceleratorCompatibility'] ==
        'true';
  }

  Future<File> original() async {
    if (!compatible) return File(executable);
    final name = await sourceRecord.readAsString();
    if (name != p.windows.basename(name) ||
        !name.toLowerCase().endsWith('.exe') ||
        name.toLowerCase() == 'chrome.exe') {
      throw StateError('原程序记录无效，请从 GBF Flash Cache.exe 启动');
    }
    final source = File(p.join(directory, name));
    if (!await source.exists()) {
      throw StateError('找不到原程序，请恢复完整发行包后重试');
    }
    return source;
  }

  Future<File?> prepare(bool enabled) async {
    if (enabled == compatible) return null;
    final source = await original();
    if (!enabled) return source;
    final target = File(p.join(directory, 'chrome.exe'));
    // Only replace a copy created by this feature, never an unrelated chrome.exe.
    final targetType = await FileSystemEntity.type(
      target.path,
      followLinks: false,
    );
    if (targetType != FileSystemEntityType.notFound &&
        (targetType != FileSystemEntityType.file ||
            !await sourceRecord.exists() ||
            await sourceRecord.readAsString() != p.basename(source.path))) {
      throw StateError('目录中已有非本功能创建的 chrome.exe，请移开该文件后重试');
    }
    // A closing process can still have its EXE mapped. Reuse an unchanged copy.
    if (targetType == FileSystemEntityType.file &&
        await source.length() == await target.length() &&
        listEquals(await source.readAsBytes(), await target.readAsBytes())) {
      return target;
    }
    final staging = await Directory(directory).createTemp('.gfc-launch-');
    try {
      final copy = await source.copy(p.join(staging.path, 'chrome.exe'));
      await sourceRecord.parent.create(recursive: true);
      final record = File(p.join(staging.path, 'windows-launcher.txt'));
      await record.writeAsString(p.basename(source.path), flush: true);
      await record.rename(sourceRecord.path);
      // Replace only after copying completes; a failed update leaves the old copy intact.
      final waiting = Stopwatch()..start();
      while (true) {
        try {
          await copy.rename(target.path);
          break;
        } on FileSystemException catch (error) {
          // Windows may keep the old image locked briefly after window teardown.
          if (!Platform.isWindows ||
              ![5, 32, 33].contains(error.osError?.errorCode) ||
              waiting.elapsed >= const Duration(seconds: 5)) {
            rethrow;
          }
          await Future<void>.delayed(const Duration(milliseconds: 100));
        }
      }
    } finally {
      await staging.delete(recursive: true);
    }
    return target;
  }

  Future<bool> launch(bool enabled) async {
    final target = await prepare(enabled);
    if (target == null) return false;
    await Process.start(
      target.path,
      ['--gfc-wait-for=$pid'],
      workingDirectory: directory,
      mode: ProcessStartMode.detached,
    );
    return true;
  }
}
