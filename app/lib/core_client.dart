import 'dart:async';
import 'dart:convert';
import 'dart:ffi';
import 'dart:isolate';

import 'package:ffi/ffi.dart';

/// One worker isolate serializes native calls without blocking the UI.
class CoreClient {
  CoreClient({this.libraryPath, this.dataDirectory});
  final String? libraryPath, dataDirectory;
  ReceivePort? _events;
  SendPort? _commands;
  final _pending = <int, Completer<Map<String, String>>>{};
  final _ready = Completer<void>(), _exited = Completer<void>();
  int _sequence = 0;
  bool _closing = false;
  void Function()? onExit;

  Future<void> connect() async {
    final library = libraryPath;
    final home = dataDirectory;
    if (library == null || home == null) {
      throw StateError('平台宿主未提供原生库或数据目录');
    }
    final events = ReceivePort();
    _events = events;
    events.listen((dynamic event) {
      if (event == null || event is List) {
        final error = StateError('缓存核心已退出');
        if (!_ready.isCompleted) _ready.completeError(error);
        for (final call in _pending.values) {
          if (!call.isCompleted) call.completeError(error);
        }
        _pending.clear();
        if (!_exited.isCompleted) _exited.complete();
        if (!_closing) onExit?.call();
        return;
      }
      final message = event as Map;
      if (message['event'] == 'ready') {
        _commands = message['port'] as SendPort;
        if (!_ready.isCompleted) _ready.complete();
      } else if (message['event'] == 'failed') {
        if (!_ready.isCompleted) {
          _ready.completeError(StateError(message['error'] as String));
        }
      } else {
        final call = _pending.remove(message['id']);
        if (call == null) return;
        if (message['ok'] == true) {
          call.complete(Map<String, String>.from(message['fields'] as Map));
        } else {
          call.completeError(StateError(message['error'] as String));
        }
      }
    });
    try {
      await Isolate.spawn(
        _nativeWorker,
        [events.sendPort, library, home],
        onError: events.sendPort,
        onExit: events.sendPort,
      );
      await _ready.future;
    } catch (_) {
      events.close();
      rethrow;
    }
  }

  Future<Map<String, String>> call(
    String op, [
    Map<String, String> args = const {},
  ]) {
    final commands = _commands;
    if (commands == null ||
        _exited.isCompleted ||
        (_closing && op != '_close')) {
      return Future.error(StateError('缓存核心未连接'));
    }
    final id = ++_sequence, result = Completer<Map<String, String>>();
    _pending[id] = result;
    commands.send({'id': id, 'op': op, 'args': args});
    return result.future;
  }

  Future<void> close() async {
    _closing = true;
    if (_events != null && !_ready.isCompleted) {
      try {
        await _ready.future;
      } catch (_) {
        _events?.close();
        return;
      }
    }
    if (_commands == null || _exited.isCompleted) {
      _events?.close();
      return;
    }
    try {
      await call('_close');
      await _exited.future;
    } finally {
      _events?.close();
      _commands = null;
    }
  }
}

typedef _OpenNative = Pointer<Void> Function(
  Pointer<Utf8>,
  Pointer<Pointer<Utf8>>,
);
typedef _Open = Pointer<Void> Function(Pointer<Utf8>, Pointer<Pointer<Utf8>>);
typedef _CallNative = Pointer<Utf8> Function(Pointer<Void>, Pointer<Utf8>);
typedef _Call = Pointer<Utf8> Function(Pointer<Void>, Pointer<Utf8>);
typedef _CloseNative = Pointer<Utf8> Function(Pointer<Void>);
typedef _Close = Pointer<Utf8> Function(Pointer<Void>);
typedef _FreeNative = Void Function(Pointer<Utf8>);
typedef _Free = void Function(Pointer<Utf8>);

Future<void> _nativeWorker(List<dynamic> startup) async {
  final parent = startup[0] as SendPort;
  Pointer<Void> handle = nullptr;
  _Close? close;
  _Free? free;
  ReceivePort? commands;
  try {
    final library = DynamicLibrary.open(startup[1] as String);
    final open = library.lookupFunction<_OpenNative, _Open>('gbf_core_open');
    final invoke = library.lookupFunction<_CallNative, _Call>(
      'gbf_core_command',
    );
    close = library.lookupFunction<_CloseNative, _Close>('gbf_core_close');
    free = library.lookupFunction<_FreeNative, _Free>('gbf_core_free_string');
    Map<String, dynamic> decode(Pointer<Utf8> text) {
      try {
        return jsonDecode(text.toDartString()) as Map<String, dynamic>;
      } finally {
        free!(text);
      }
    }

    final home = (startup[2] as String).toNativeUtf8();
    final error = calloc<Pointer<Utf8>>();
    try {
      handle = open(home, error);
      if (handle == nullptr) {
        final result = decode(error.value);
        throw StateError(result['error'] as String);
      }
    } finally {
      calloc.free(error);
      calloc.free(home);
    }
    commands = ReceivePort();
    parent.send({'event': 'ready', 'port': commands.sendPort});
    await for (final dynamic event in commands) {
      final command = event as Map;
      if (command['op'] == '_close') {
        final output = close(handle);
        handle = nullptr;
        final result = decode(output);
        parent.send({'id': command['id'], ...result});
        break;
      }
      final input = jsonEncode({'op': command['op'], 'args': command['args']})
          .toNativeUtf8();
      try {
        parent.send({'id': command['id'], ...decode(invoke(handle, input))});
      } finally {
        calloc.free(input);
      }
    }
  } catch (error) {
    parent.send({'event': 'failed', 'error': error.toString()});
  } finally {
    if (handle != nullptr && close != null && free != null) free(close(handle));
    commands?.close();
  }
}
