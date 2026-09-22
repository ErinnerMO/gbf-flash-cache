import 'dart:io';

import 'package:gbf_flash_cache/core_client.dart';

// Local ABI integration check; does not contact game servers.
Future<void> main(List<String> arguments) async {
  if (arguments.length != 1) {
    throw ArgumentError('Pass the native library path');
  }
  final home = await Directory.systemTemp.createTemp('gbf-native-中文-');
  final probe = await ServerSocket.bind(InternetAddress.loopbackIPv4, 0);
  final port = probe.port;
  await probe.close();
  final client = CoreClient(
    libraryPath: File(arguments.single).absolute.path,
    dataDirectory: home.path,
  );
  void check(bool condition, String message) {
    if (!condition) throw StateError(message);
  }

  try {
    await client.connect();
    final manifest = await File.fromUri(
      Platform.script.resolve('../pubspec.yaml'),
    ).readAsString();
    final version = RegExp(
      r'^version: ([^+\s]+)',
      multiLine: true,
    ).firstMatch(manifest)!.group(1);
    check((await client.call('init'))['version'] == version, 'version');
    final ca = await client.call('ca');
    check(await File(ca['path']!).exists(), 'CA export');
    await client.call('start', {'port': '$port', 'memoryMiB': '128'});
    check((await client.call('status'))['running'] == 'true', 'start');
    final socket = await Socket.connect(InternetAddress.loopbackIPv4, port);
    socket.destroy();
    await client.call('stop');
    check((await client.call('status'))['running'] == 'false', 'stop');
    await client.call('clear');
    check((await client.call('status'))['diskBytes'] == '0', 'clear');
    stdout.writeln(
      'Native Flutter bridge: lifecycle, CA, listener, clear passed',
    );
  } finally {
    await client.close();
    await home.delete(recursive: true);
  }
}
