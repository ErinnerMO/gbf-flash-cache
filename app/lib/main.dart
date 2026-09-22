import 'protocol_selector.dart';
import 'help_guide.dart';

import 'dart:async';
import 'dart:io';
import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:path/path.dart' as p;
import 'package:file_selector/file_selector.dart';
import 'package:window_manager/window_manager.dart';
import 'package:tray_manager/tray_manager.dart';
import 'package:screen_retriever/screen_retriever.dart';

import 'core_client.dart';
import 'platform/windows.dart';
import 'mobile_dashboard.dart';

const ink = Color(0xFF181818),
    surface = Color(0xFF1F1F1F),
    line = Color(0xFF2B2B2B);
const textPrimary = Color(0xFFCCCCCC), textSecondary = Color(0xFFCCCCCC);
const muted = Color(0xFF9D9D9D),
    mint = Color(0xFF5AB5A3),
    gold = Color(0xFFE2C08D);
const appVersion = String.fromEnvironment(
  'GBF_VERSION',
  defaultValue: '1.14.98',
);

Future<void> main() async {
  WidgetsFlutterBinding.ensureInitialized();
  if (Platform.isAndroid) {
    runApp(const FlashApp());
    return;
  }
  if (Platform.isWindows) {
    try {
      if (await WindowsHost.launcher.launch(
        await WindowsHost.launcher.enabled(),
      )) {
        exit(0);
      }
    } catch (e) {
      WindowsHost.startupWarning = '加速器兼容模式切换失败，已保留当前模式：$e';
    }
  }
  await windowManager.ensureInitialized();
  final display = await screenRetriever.getPrimaryDisplay();
  final available = display.visibleSize ?? display.size;
  final size = Size(
    math.min(1120, available.width - 32),
    math.min(860, available.height - 32),
  );
  await windowManager.waitUntilReadyToShow(
    WindowOptions(
      size: size,
      minimumSize: Size(math.min(860, size.width), math.min(660, size.height)),
      center: true,
      title: 'GBF Flash Cache',
      titleBarStyle: TitleBarStyle.hidden,
      windowButtonVisibility: false,
      backgroundColor: ink,
    ),
    () async {},
  );
  runApp(const FlashApp());
}

String displayPath(String value) {
  if (value.startsWith(r'\\?\UNC\')) return r'\\' + value.substring(8);
  if (value.startsWith(r'\\?\')) return value.substring(4);
  return value;
}

// VS Code Modern neutral palette with GFC teal accents.
ThemeData appTheme(Brightness brightness) {
  final dark = brightness == Brightness.dark;
  final ink = dark ? const Color(0xFF181818) : const Color(0xFFF8F8F8);
  final surface = dark ? const Color(0xFF1F1F1F) : Colors.white;
  final line = dark ? const Color(0xFF2B2B2B) : const Color(0xFFE5E5E5);
  final textPrimary = dark ? const Color(0xFFCCCCCC) : const Color(0xFF3B3B3B);
  final textSecondary = dark
      ? const Color(0xFFCCCCCC)
      : const Color(0xFF3B3B3B);
  final muted = dark ? const Color(0xFF9D9D9D) : const Color(0xFF616161);
  final mint = dark ? const Color(0xFF5AB5A3) : const Color(0xFF177D70);
  final offSwitchColor = WidgetStateProperty.resolveWith<Color?>(
    (states) =>
        states.contains(WidgetState.disabled) ||
            states.contains(WidgetState.selected)
        ? null
        : muted,
  );
  return ThemeData(
    brightness: brightness,
    useMaterial3: true,
    switchTheme: SwitchThemeData(
      thumbColor: offSwitchColor,
      trackOutlineColor: offSwitchColor,
    ),
    scaffoldBackgroundColor: ink,
    colorScheme: (dark ? const ColorScheme.dark() : const ColorScheme.light())
        .copyWith(
          primary: mint,
          onPrimary: dark ? ink : Colors.white,
          secondary: mint,
          onSecondary: dark ? ink : Colors.white,
          onError: dark ? Colors.black : Colors.white,
          surface: surface,
          onSurface: textPrimary,
          onSurfaceVariant: textSecondary,
          outlineVariant: line,
          primaryContainer: dark
              ? const Color(0xFF313131)
              : const Color(0xFFDCEAE5),
          onPrimaryContainer: textPrimary,
          secondaryContainer: dark
              ? const Color(0xFF313131)
              : const Color(0xFFDCEAE5),
          onSecondaryContainer: textPrimary,
          error: dark ? const Color(0xFFFFA6A6) : const Color(0xFFB3261E),
          outline: dark ? const Color(0xFF3C3C3C) : const Color(0xFFCECECE),
          surfaceContainer: surface,
          surfaceContainerHighest: dark
              ? const Color(0xFF313131)
              : const Color(0xFFE5E5E5),
          surfaceTint: Colors.transparent,
        ),
    fontFamily: Platform.isWindows ? 'Microsoft YaHei UI' : null,
    fontFamilyFallback: const [
      'Microsoft YaHei UI',
      'Noto Sans CJK SC',
      'sans-serif',
    ],
    textTheme: TextTheme(
      bodyLarge: TextStyle(fontSize: 16, height: 1.35, color: textPrimary),
      bodyMedium: TextStyle(fontSize: 15, height: 1.5, color: textSecondary),
      bodySmall: TextStyle(fontSize: 14, height: 1.45, color: muted),
      titleLarge: TextStyle(
        fontSize: 22,
        fontWeight: FontWeight.w600,
        color: textPrimary,
      ),
      titleMedium: TextStyle(
        fontSize: 16,
        fontWeight: FontWeight.w600,
        color: textPrimary,
      ),
      labelLarge: TextStyle(fontSize: 14, fontWeight: FontWeight.w600),
    ),
    textButtonTheme: TextButtonThemeData(
      style: TextButton.styleFrom(
        foregroundColor: mint,
        minimumSize: const Size(80, 44),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
        padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 12),
      ),
    ),
    filledButtonTheme: FilledButtonThemeData(
      style: FilledButton.styleFrom(
        minimumSize: const Size(80, 44),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
        padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 12),
      ),
    ),
    outlinedButtonTheme: OutlinedButtonThemeData(
      style: OutlinedButton.styleFrom(
        minimumSize: const Size(80, 44),
        shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
        side: BorderSide(color: line),
      ),
    ),
    listTileTheme: ListTileThemeData(
      contentPadding: const EdgeInsets.symmetric(horizontal: 12, vertical: 4),
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
    ),
    dividerColor: line,
    dividerTheme: DividerThemeData(color: line),
    visualDensity: VisualDensity.standard,
    inputDecorationTheme: InputDecorationTheme(
      filled: true,
      fillColor: dark ? const Color(0xFF313131) : Colors.white,
      helperStyle: TextStyle(fontSize: 14, color: muted),
      contentPadding: const EdgeInsets.symmetric(horizontal: 16, vertical: 14),
      border: OutlineInputBorder(
        borderRadius: BorderRadius.circular(12),
        borderSide: BorderSide.none,
      ),
      enabledBorder: OutlineInputBorder(
        borderRadius: BorderRadius.circular(12),
        borderSide: BorderSide(
          color: dark ? const Color(0xFF3C3C3C) : const Color(0xFFCECECE),
        ),
      ),
    ),
    popupMenuTheme: PopupMenuThemeData(
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(12)),
      color: surface,
      surfaceTintColor: Colors.transparent,
    ),
    dialogTheme: DialogThemeData(
      backgroundColor: surface,
      shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(24)),
    ),
    tooltipTheme: const TooltipThemeData(
      waitDuration: Duration(milliseconds: 350),
    ),
  );
}

class FlashApp extends StatefulWidget {
  const FlashApp({super.key, this.client});
  final CoreClient? client;
  @override
  State<FlashApp> createState() => _FlashAppState();
}

class _FlashAppState extends State<FlashApp> {
  ThemeMode mode = ThemeMode.dark;
  @override
  Widget build(BuildContext context) => MaterialApp(
    debugShowCheckedModeBanner: false,
    title: 'GBF Flash Cache',
    theme: appTheme(Brightness.light),
    darkTheme: appTheme(Brightness.dark),
    themeMode: mode,
    home: Platform.isAndroid
        ? MobileDashboard(
            onThemeChanged: (value) => setState(() => mode = value),
          )
        : Dashboard(
            client: widget.client,
            onThemeChanged: (value) => setState(() => mode = value),
          ),
  );
}

class Dashboard extends StatefulWidget {
  const Dashboard({super.key, this.client, this.onThemeChanged});
  final ValueChanged<ThemeMode>? onThemeChanged;
  final CoreClient? client;
  @override
  State<Dashboard> createState() => _DashboardState();
}

class _DashboardState extends State<Dashboard>
    with WindowListener, TrayListener {
  Color get gold => Theme.of(context).brightness == Brightness.dark
      ? const Color(0xFFE2C08D)
      : const Color(0xFF895503);
  Color get errorColor => Theme.of(context).colorScheme.error;
  ThemeMode themeMode = ThemeMode.dark;
  Color get ink => Theme.of(context).scaffoldBackgroundColor;
  Color get surface => Theme.of(context).colorScheme.surface;
  Color get line => Theme.of(context).dividerColor;
  Color get textPrimary => Theme.of(context).colorScheme.onSurface;
  Color get textSecondary => Theme.of(context).textTheme.bodyMedium!.color!;
  Color get muted => Theme.of(context).textTheme.bodySmall!.color!;
  Color get mint => Theme.of(context).colorScheme.primary;
  late final CoreClient core;
  final port = TextEditingController(text: '8765'),
      host = TextEditingController(text: '127.0.0.1'),
      proxyPort = TextEditingController(text: '7890'),
      username = TextEditingController(),
      password = TextEditingController(),
      memory = TextEditingController(text: '128');
  bool ready = false,
      running = false,
      busy = false,
      proxy = false,
      keepLogs = false,
      lan = false,
      autoStart = false,
      autoRun = false,
      startHidden = false,
      closeToTray = true,
      acceleratorCompatibility = false,
      polling = false,
      trayReady = false,
      exiting = false;
  String protocol = 'HTTP',
      home = '',
      error = '',
      cachePath = '',
      logsPath = '';
  bool starting = false, startFailed = false, passwordUnavailable = false;
  String directoryWarning = '';
  String get visibleError =>
      [error, directoryWarning].where((s) => s.isNotEmpty).join('\n');
  int page = 0;
  Map<String, String> stats = {};
  Timer? timer;
  final messenger = GlobalKey<ScaffoldMessengerState>();

  @override
  void initState() {
    super.initState();
    core = widget.client ?? WindowsHost.createClient();
    windowManager.addListener(this);
    trayManager.addListener(this);
    unawaited(initialize());
  }

  Future<void> initialize() async {
    core.onExit = () {
      if (mounted && !exiting) {
        setState(() {
          ready = false;
          running = false;
          error = '缓存服务已退出，请重新打开应用。';
        });
      }
    };
    try {
      await core.connect();
      if (exiting) return;
      final saved = await core.call('init');
      if (!mounted || exiting) return;
      setState(() {
        for (final entry in {
          'port': port,
          'host': host,
          'proxyPort': proxyPort,
          'username': username,
          'password': password,
          'memoryMiB': memory,
        }.entries) {
          if (saved.containsKey(entry.key)) {
            entry.value.text = saved[entry.key]!;
          }
        }
        error = [
          WindowsHost.startupWarning,
          saved['passwordWarning'] ?? saved['startupWarning'] ?? '',
        ].where((value) => value.isNotEmpty).join('\n');
        passwordUnavailable = saved.containsKey('passwordWarning');
        directoryWarning = saved['directoryWarning'] ?? '';
        protocol = saved['protocol'] ?? 'HTTP';
        proxy = saved['proxy'] == 'true';
        keepLogs = saved['keepLogs'] == 'true';
        lan = saved['lan'] == 'true';
        autoStart = saved['autoStart'] == 'true';
        autoRun = saved['autoRun'] == 'true';
        startHidden = saved['startHidden'] == 'true';
        closeToTray = saved['closeToTray'] != 'false';
        acceleratorCompatibility = saved['acceleratorCompatibility'] == 'true';
        home = saved['home']!;
        cachePath = saved['cachePath'] ?? p.join(home, 'cache');
        logsPath = saved['logsPath'] ?? p.join(home, 'logs');
        ready = true;
      });
      themeMode = ThemeMode.values.firstWhere(
        (v) => v.name == saved['themeMode'],
        orElse: () => ThemeMode.dark,
      );
      widget.onThemeChanged?.call(themeMode);
      await refresh();
      if (exiting) return;
      timer = Timer.periodic(const Duration(seconds: 3), (_) {
        if (!busy) unawaited(refresh());
      });
      await windowManager.setPreventClose(true);
    } catch (e) {
      if (mounted) setState(() => error = '无法启动缓存服务：$e');
    }
    if (exiting) return;
    try {
      await trayManager.setIcon(
        Platform.isWindows ? WindowsHost.trayIcon : 'assets/app-icon.png',
      );
      await trayManager.setToolTip('GBF Flash Cache');
      await trayManager.setContextMenu(
        Menu(
          items: [
            MenuItem(key: 'open', label: '打开窗口'),
            MenuItem.separator(),
            MenuItem(key: 'exit', label: '退出'),
          ],
        ),
      );
      trayReady = true;
    } catch (_) {
      /* A missing desktop tray must not hide the only window. */
    }
    await windowManager.setPreventClose(true);
    if (ready && autoRun && visibleError.isEmpty) await toggle();
    if (!exiting &&
        (!startHidden || !trayReady || !ready || visibleError.isNotEmpty)) {
      await windowManager.show();
      await windowManager.focus();
    }
  }

  Future<void> refresh() async {
    if (!ready || polling || exiting) return;
    polling = true;
    try {
      final result = await core.call('status');
      if (mounted) {
        setState(() {
          stats = result;
          directoryWarning = result['directoryWarning'] ?? '';
          running = result['running'] == 'true';
        });
      }
    } catch (e) {
      if (mounted) setState(() => error = '$e');
    } finally {
      polling = false;
    }
  }

  Map<String, String> settings() => {
    'port': port.text.trim(),
    'proxy': '$proxy',
    'protocol': protocol,
    'host': host.text.trim(),
    'proxyPort': proxyPort.text.trim(),
    'username': username.text,
    if (!passwordUnavailable) 'password': password.text,
    'memoryMiB': memory.text.trim(),
    'keepLogs': '$keepLogs',
    'lan': '$lan',
    'autoRun': '$autoRun',
    'startHidden': '$startHidden',
    'closeToTray': '$closeToTray',
  };
  Future<void> perform(Future<void> Function() action) async {
    if (busy || !ready || exiting) return;
    setState(() {
      busy = true;
      error = '';
    });
    try {
      await action();
      await refresh();
    } catch (e) {
      if (mounted) {
        setState(() => error = e.toString().replaceFirst('Bad state: ', ''));
      }
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  Future<void> toggle() async {
    if (busy) return;
    final wasRunning = running;
    setState(() {
      starting = !wasRunning;
      startFailed = false;
    });
    await perform(() async {
      await core.call(wasRunning ? 'stop' : 'start', settings());
    });
    if (mounted) {
      setState(() {
        starting = false;
        startFailed = !wasRunning && !running && error.isNotEmpty;
      });
    }
  }

  Future<void> reveal(String path) => WindowsHost.reveal(path);

  Future<bool> confirm(String title, String message) async =>
      await showDialog<bool>(
        context: context,
        builder: (c) => AlertDialog(
          titlePadding: const EdgeInsets.fromLTRB(28, 24, 28, 24),
          contentPadding: const EdgeInsets.fromLTRB(28, 0, 28, 8),
          actionsPadding: const EdgeInsets.fromLTRB(20, 16, 24, 24),
          title: Text(title),
          content: Text(message),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(c, false),
              child: const Text('取消'),
            ),
            FilledButton(
              onPressed: () => Navigator.pop(c, true),
              child: const Text('确认'),
            ),
          ],
        ),
      ) ??
      false;
  Future<void> exportLogs() => perform(() async {
    const zip = XTypeGroup(label: 'ZIP 日志', extensions: ['zip']);
    final result = await getSaveLocation(
      suggestedName: 'gbf-logs-${DateTime.now().millisecondsSinceEpoch}.zip',
      acceptedTypeGroups: [zip],
    );
    if (result == null) return;
    await core.call('export', {'path': result.path});
    toast('日志已导出');
  });
  void toast(String text) => messenger.currentState?.showSnackBar(
    SnackBar(content: Text(text), behavior: SnackBarBehavior.floating),
  );
  Future<void> caSettings() async {
    Map<String, String> state = {};
    String failure = '';
    bool working = false;
    try {
      state = await core.call('ca_status');
      failure = state['warning'] ?? '';
    } catch (e) {
      toast('无法读取 CA 证书：$e');
      return;
    }
    if (!mounted) return;
    await showDialog<void>(
      context: context,
      barrierDismissible: false,
      builder: (dialog) => StatefulBuilder(
        builder: (c, update) {
          Future<void> act(String op) async {
            update(() {
              working = true;
              failure = '';
            });
            try {
              if (op == 'ca_regenerate' || op == 'ca_uninstall') {
                if (running) await core.call('stop');
              }
              final result = await core.call(op, {
                'fingerprint': state['fingerprint'] ?? '',
              });
              if (c.mounted) {
                update(() {
                  state = result;
                  failure = result['warning'] ?? '';
                });
              }
              await refresh();
            } catch (e) {
              if (c.mounted) {
                update(
                  () => failure = e.toString().replaceFirst('Bad state: ', ''),
                );
              }
            } finally {
              if (c.mounted) update(() => working = false);
            }
          }

          final valid = state['state'] == 'valid';
          final present =
              state['state'] != 'missing' && state['state'] != 'loading';
          final trusted = state['trusted'] == 'true';
          return PopScope(
            canPop: !working,
            child: AlertDialog(
              titlePadding: const EdgeInsets.fromLTRB(28, 24, 28, 24),
              contentPadding: const EdgeInsets.fromLTRB(28, 0, 28, 8),
              actionsPadding: const EdgeInsets.fromLTRB(20, 16, 24, 24),
              title: const Text('CA 证书'),
              content: SizedBox(
                width: 560,
                child: SingleChildScrollView(
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      if (working) const LinearProgressIndicator(),
                      const SizedBox(height: 12),
                      Row(
                        children: [
                          Expanded(
                            child: label(
                              'GBF Flash Cache CA',
                              color: textPrimary,
                              size: 17,
                              weight: FontWeight.w600,
                            ),
                          ),
                          label(
                            valid
                                ? '有效'
                                : present
                                ? '无效或已过期'
                                : '未生成',
                            color: valid ? mint : gold,
                          ),
                        ],
                      ),
                      const SizedBox(height: 26),
                      Row(
                        children: [
                          Expanded(
                            child: label('Windows 系统信任', color: textSecondary),
                          ),
                          label(
                            state['state'] == 'missing'
                                ? '未安装'
                                : trusted
                                ? '已安装'
                                : state['trusted'] == 'false'
                                ? '未安装'
                                : '无法确认',
                            color: trusted ? mint : muted,
                          ),
                        ],
                      ),
                      if (state['notAfter'] != null) ...[
                        const Divider(height: 32),
                        label('有效期'),
                        const SizedBox(height: 8),
                        Text("${state['notBefore']} — ${state['notAfter']}"),
                      ],
                      const SizedBox(height: 18),
                      label('使用独立证书库的浏览器需单独安装'),
                      if (state['fingerprint'] != null)
                        ExpansionTile(
                          tilePadding: EdgeInsets.zero,
                          title: label('证书详情'),
                          children: [
                            SelectableText(
                              "SHA-256\n${state['fingerprint']}",
                              style: TextStyle(fontSize: 13, color: muted),
                            ),
                            TextButton(
                              onPressed: working
                                  ? null
                                  : () async {
                                      try {
                                        final result = await core.call('ca');
                                        await reveal(
                                          p.dirname(result['path']!),
                                        );
                                      } catch (e) {
                                        if (c.mounted) {
                                          update(() => failure = '$e');
                                        }
                                      }
                                    },
                              child: const Text('打开证书目录'),
                            ),
                          ],
                        ),
                      const SizedBox(height: 16),
                      Row(
                        children: [
                          OutlinedButton(
                            onPressed: working || (present && !valid)
                                ? null
                                : () async {
                                    if (await confirm(
                                      '安装 CA 证书',
                                      '将本应用的 CA 安装到当前 Windows 用户的受信任根证书中。',
                                    )) {
                                      if (c.mounted) await act('ca_install');
                                    }
                                  },
                            child: Text(trusted ? '重新安装' : '安装证书'),
                          ),
                          const Spacer(),
                          TextButton(
                            onPressed: working || !trusted
                                ? null
                                : () async {
                                    if (await confirm(
                                      '卸载 CA 证书信任',
                                      '停止缓存服务并移除当前 CA 的 Windows 信任。本地证书文件保留。',
                                    )) {
                                      if (c.mounted) await act('ca_uninstall');
                                    }
                                  },
                            child: const Text('卸载信任'),
                          ),
                        ],
                      ),
                      if (present) ...[
                        const Divider(height: 32),
                        Row(
                          children: [
                            Expanded(
                              child: label('更换本地证书', color: textSecondary),
                            ),
                            OutlinedButton(
                              onPressed: working
                                  ? null
                                  : () async {
                                      if (await confirm(
                                        '重新生成 CA 证书？',
                                        '将停止缓存服务，并在新证书生成成功后替换现有 CA。\n\n所有使用旧 CA 的浏览器和设备都需要安装新证书。',
                                      )) {
                                        if (c.mounted) {
                                          await act('ca_regenerate');
                                        }
                                      }
                                    },
                              child: Text(
                                '重新生成',
                                style: TextStyle(color: errorColor),
                              ),
                            ),
                          ],
                        ),
                      ],
                      if (failure.isNotEmpty)
                        Padding(
                          padding: const EdgeInsets.only(top: 18),
                          child: Text(
                            failure,
                            style: TextStyle(color: errorColor),
                          ),
                        ),
                    ],
                  ),
                ),
              ),
              actions: [
                FilledButton(
                  onPressed: working ? null : () => Navigator.pop(c),
                  child: const Text('完成'),
                ),
              ],
            ),
          );
        },
      ),
    );
  }

  Future<void> setPreference(String key, bool value) => perform(() async {
    final args = {key: '$value'};
    if (key == 'autoStart') {
      await core.call('startup', {'enabled': '$value'});
    } else {
      await core.call('settings', args);
    }
    if (mounted) {
      setState(() {
        switch (key) {
          case 'autoStart':
            autoStart = value;
          case 'proxy':
            proxy = value;
          case 'keepLogs':
            keepLogs = value;
          case 'autoRun':
            autoRun = value;
          case 'startHidden':
            startHidden = value;
          case 'closeToTray':
            closeToTray = value;
          case 'acceleratorCompatibility':
            acceleratorCompatibility = value;
        }
      });
    }
  });
  Future<void> setLan(bool value) => perform(() async {
    final args = {...settings(), 'lan': '$value'};
    if (running) {
      await core.call('stop');
      try {
        await core.call('start', args);
      } catch (_) {
        await refresh();
        rethrow;
      }
    } else {
      await core.call('settings', {'lan': '$value'});
    }
    if (mounted) setState(() => lan = value);
  });
  Widget preferencesPage() {
    Widget option(String title, String key, bool value) => SwitchListTile(
      contentPadding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
      title: Text(title),
      value: value,
      onChanged: ready && !busy
          ? (v) => unawaited(setPreference(key, v))
          : null,
    );
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        label('外观', color: textSecondary, weight: FontWeight.w600),
        const SizedBox(height: 14),
        card(
          Row(
            children: [
              const Expanded(child: Text('主题')),
              PopupMenuButton<ThemeMode>(
                key: const ValueKey('theme-menu'),
                tooltip: '选择主题',
                borderRadius: BorderRadius.circular(12),
                position: PopupMenuPosition.under,
                enabled: ready && !busy,
                itemBuilder: (_) => const [
                  PopupMenuItem(value: ThemeMode.system, child: Text('跟随系统')),
                  PopupMenuItem(value: ThemeMode.light, child: Text('亮色')),
                  PopupMenuItem(value: ThemeMode.dark, child: Text('暗色')),
                ],
                onSelected: (value) async {
                  if (value == themeMode) return;
                  await perform(() async {
                    await core.call('settings', {'themeMode': value.name});
                    if (!mounted) return;
                    setState(() => themeMode = value);
                    widget.onThemeChanged?.call(value);
                  });
                },
                child: Padding(
                  padding: const EdgeInsets.symmetric(
                    horizontal: 12,
                    vertical: 12,
                  ),
                  child: Row(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Text(switch (themeMode) {
                        ThemeMode.system => '跟随系统',
                        ThemeMode.light => '亮色',
                        ThemeMode.dark => '暗色',
                      }),
                      const Icon(Icons.arrow_drop_down),
                    ],
                  ),
                ),
              ),
            ],
          ),
        ),
        const SizedBox(height: 28),
        label('启动', color: textSecondary, weight: FontWeight.w600),
        const SizedBox(height: 14),
        card(
          Column(
            children: [
              option('开机启动', 'autoStart', autoStart),
              const Divider(height: 1),
              option('启动后自动开启缓存服务', 'autoRun', autoRun),
              const Divider(height: 1),
              option('启动时隐藏到托盘', 'startHidden', startHidden),
            ],
          ),
        ),
        const SizedBox(height: 28),
        card(
          Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              SwitchListTile(
                contentPadding: const EdgeInsets.symmetric(
                  horizontal: 12,
                  vertical: 10,
                ),
                title: const Text('加速器兼容模式'),
                subtitle: const Text(
                  '以 chrome.exe 进程名运行，使按进程名识别的游戏加速器能够接管流量。切换需要重启应用。',
                ),
                value: acceleratorCompatibility,
                onChanged: ready && !busy
                    ? (value) => unawaited(
                        setPreference('acceleratorCompatibility', value),
                      )
                    : null,
              ),
              if (acceleratorCompatibility != WindowsHost.launcher.compatible)
                TextButton(
                  onPressed: ready && !busy
                      ? () => unawaited(
                          perform(() async {
                            if (await WindowsHost.launcher.launch(
                              acceleratorCompatibility,
                            )) {
                              await quit();
                            }
                          }),
                        )
                      : null,
                  child: const Text('立即重启'),
                ),
            ],
          ),
        ),
        const SizedBox(height: 28),
        label('窗口', color: textSecondary, weight: FontWeight.w600),
        const SizedBox(height: 14),
        card(option('关闭窗口时隐藏到托盘', 'closeToTray', closeToTray)),
        const SizedBox(height: 18),
        label('设置自动保存'),
      ],
    );
  }

  Future<void> quit() async {
    if (exiting) return;
    exiting = true;
    timer?.cancel();
    await windowManager.hide();
    try {
      await core.close();
    } finally {
      if (trayReady) await trayManager.destroy();
      await windowManager.destroy();
    }
  }

  @override
  void onWindowClose() {
    if (closeToTray && trayReady) {
      unawaited(windowManager.hide());
    } else {
      unawaited(quit());
    }
  }

  @override
  void onWindowMinimize() {
    if (trayReady) unawaited(windowManager.hide());
  }

  @override
  void onTrayIconMouseDown() {
    unawaited(windowManager.show());
    unawaited(windowManager.focus());
  }

  @override
  void onTrayIconRightMouseDown() {
    unawaited(trayManager.popUpContextMenu());
  }

  @override
  void onTrayMenuItemClick(MenuItem item) {
    if (item.key == 'exit') {
      unawaited(quit());
    } else {
      onTrayIconMouseDown();
    }
  }

  @override
  void dispose() {
    timer?.cancel();
    windowManager.removeListener(this);
    trayManager.removeListener(this);
    for (final c in [port, host, proxyPort, username, password, memory]) {
      c.dispose();
    }
    super.dispose();
  }

  Widget label(
    String text, {
    Color? color,
    double size = 14,
    FontWeight weight = FontWeight.w400,
  }) => Text(
    text,
    style: TextStyle(color: color ?? muted, fontSize: size, fontWeight: weight),
  );
  Widget card(Widget child, {EdgeInsets padding = const EdgeInsets.all(20)}) =>
      Material(
        color: surface,
        shape: RoundedRectangleBorder(
          side: BorderSide(color: line),
          borderRadius: BorderRadius.circular(20),
        ),
        child: Padding(padding: padding, child: child),
      );
  Widget nav(IconData icon, String hint, int index) => Padding(
    padding: const EdgeInsets.symmetric(vertical: 8),
    child: Tooltip(
      message: hint,
      child: IconButton(
        onPressed: () => setState(() => page = index),
        icon: Icon(icon, size: 23),
        style: IconButton.styleFrom(
          backgroundColor: page == index
              ? Theme.of(context).colorScheme.secondaryContainer
              : Colors.transparent,
          foregroundColor: page == index ? mint : muted,
          padding: const EdgeInsets.all(15),
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(15),
          ),
        ),
      ),
    ),
  );

  @override
  Widget build(BuildContext context) => ScaffoldMessenger(
    key: messenger,
    child: Scaffold(
      appBar: PreferredSize(
        preferredSize: const Size.fromHeight(38),
        child: SizedBox(
          height: 38,
          child: WindowCaption(
            backgroundColor: ink,
            brightness: Theme.of(context).brightness,
            title: Row(
              children: [
                Image.asset('assets/app-icon.png', width: 22, height: 22),
                const SizedBox(width: 10),
                const Text('GBF Flash Cache'),
              ],
            ),
          ),
        ),
      ),
      body: Row(
        children: [
          Container(
            width: 82,
            decoration: BoxDecoration(
              color: surface,
              border: Border(right: BorderSide(color: line)),
            ),
            child: Column(
              children: [
                const SizedBox(height: 16),
                Image.asset('assets/app-icon.png', width: 44, height: 44),
                const SizedBox(height: 38),
                nav(Icons.grid_view_rounded, '服务', 0),
                nav(Icons.menu_book_rounded, '使用说明', 1),
                const Spacer(),
                nav(Icons.settings_outlined, '设置', 2),
                IconButton(
                  tooltip: '关于',
                  onPressed: about,
                  icon: Icon(
                    Icons.info_outline_rounded,
                    color: muted,
                    size: 21,
                  ),
                ),
                const SizedBox(height: 16),
              ],
            ),
          ),
          Expanded(
            child: SingleChildScrollView(
              padding: const EdgeInsets.fromLTRB(28, 20, 28, 16),
              child: Center(
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 1120),
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Row(
                        children: [
                          Column(
                            crossAxisAlignment: CrossAxisAlignment.start,
                            children: [
                              label(
                                page == 2 ? '设置' : 'GBF Flash Cache',
                                color: textPrimary,
                                size: 23,
                                weight: FontWeight.w700,
                              ),
                              const SizedBox(height: 5),
                              label(
                                page == 0
                                    ? 'GBF 资源加载加速'
                                    : page == 2
                                    ? '启动与窗口行为'
                                    : '几步设置，开始使用。',
                              ),
                            ],
                          ),
                          const Spacer(),
                          Container(
                            padding: const EdgeInsets.symmetric(
                              horizontal: 12,
                              vertical: 7,
                            ),
                            decoration: BoxDecoration(
                              color: Theme.of(context)
                                  .colorScheme
                                  .surfaceContainerHighest,
                              borderRadius: BorderRadius.circular(30),
                            ),
                            child: Row(
                              children: [
                                Icon(
                                  Icons.circle,
                                  size: 7,
                                  color: running ? mint : muted,
                                ),
                                const SizedBox(width: 8),
                                label(
                                  !ready
                                      ? '核心未连接'
                                      : running
                                      ? '服务运行中'
                                      : '服务已停止',
                                  color: running ? mint : muted,
                                  size: 12,
                                ),
                              ],
                            ),
                          ),
                        ],
                      ),
                      const SizedBox(height: 16),
                      if (visibleError.isNotEmpty)
                        Padding(
                          padding: const EdgeInsets.only(bottom: 16),
                          child: Container(
                            padding: const EdgeInsets.all(16),
                            decoration: BoxDecoration(
                              color: Theme.of(context)
                                  .colorScheme
                                  .surfaceContainerHighest,
                              borderRadius: BorderRadius.circular(14),
                            ),
                            child: Row(
                              children: [
                                Icon(Icons.error_outline, color: errorColor),
                                const SizedBox(width: 12),
                                Expanded(child: Text(visibleError)),
                              ],
                            ),
                          ),
                        ),
                      if (page == 0)
                        ...servicePage()
                      else if (page == 2)
                        preferencesPage()
                      else
                        helpPage(),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ],
      ),
    ),
  );

  List<Widget> servicePage() => [
    Container(
      padding: const EdgeInsets.all(20),
      decoration: BoxDecoration(
        color: surface,
        borderRadius: BorderRadius.circular(24),
        border: Border.all(color: line),
      ),
      child: Row(
        children: [
          Container(
            width: 68,
            height: 68,
            decoration: BoxDecoration(
              shape: BoxShape.circle,
              color: Theme.of(context).colorScheme.surfaceContainerHighest,
              border: Border.all(color: line),
            ),
            child: Icon(
              Icons.power_settings_new_rounded,
              color: startFailed
                  ? errorColor
                  : running
                  ? mint
                  : muted,
              size: 34,
            ),
          ),
          const SizedBox(width: 22),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                label(
                  starting
                      ? '启动中'
                      : running
                      ? '运行中'
                      : startFailed
                      ? '启动失败'
                      : '未启动',
                  color: startFailed
                      ? errorColor
                      : running
                      ? mint
                      : textPrimary,
                  size: 23,
                  weight: FontWeight.w600,
                ),
                const SizedBox(height: 8),
                Row(
                  children: [
                    label(
                      '127.0.0.1:${port.text}',
                      color: textSecondary,
                      size: 14,
                    ),
                    const SizedBox(width: 6),
                    IconButton(
                      tooltip: '复制代理地址',
                      onPressed: () {
                        Clipboard.setData(
                          ClipboardData(text: '127.0.0.1:${port.text}'),
                        );
                        toast('代理地址已复制');
                      },
                      icon: Icon(Icons.copy_rounded, size: 15, color: muted),
                      constraints: const BoxConstraints(),
                      padding: const EdgeInsets.all(5),
                    ),
                  ],
                ),
              ],
            ),
          ),
          FilledButton.icon(
            onPressed: ready && !busy ? toggle : null,
            style: FilledButton.styleFrom(
              backgroundColor: running ? line : mint,
              foregroundColor: running
                  ? textPrimary
                  : Theme.of(context).colorScheme.onPrimary,
              padding: const EdgeInsets.symmetric(horizontal: 26, vertical: 22),
              shape: RoundedRectangleBorder(
                borderRadius: BorderRadius.circular(15),
              ),
            ),
            icon: busy
                ? const SizedBox(
                    width: 17,
                    height: 17,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  )
                : Icon(running ? Icons.stop_rounded : Icons.play_arrow_rounded),
            label: Text(
              busy
                  ? '请稍候'
                  : running
                  ? '停止服务'
                  : '启动服务',
              style: const TextStyle(fontWeight: FontWeight.w700),
            ),
          ),
        ],
      ),
    ),
    const SizedBox(height: 16),
    Row(
      children: [
        metric(
          '预载完成',
          stats['preloaded'] ?? '0',
          Icons.download_done_rounded,
          mint,
        ),
        const SizedBox(width: 16),
        metric('资源回源', stats['requests'] ?? '0', Icons.public_rounded, mint),
        const SizedBox(width: 16),
        metric('缓存命中', stats['hits'] ?? '0', Icons.flash_on_rounded, mint),
        const SizedBox(width: 16),
        metric(
          '网络异常',
          stats['failures'] ?? '0',
          Icons.error_outline_rounded,
          (int.tryParse(stats['failures'] ?? '') ?? 0) > 0 ? errorColor : muted,
        ),
      ],
    ),
    const SizedBox(height: 16),
    LayoutBuilder(
      builder: (context, bounds) {
        final wide = bounds.maxWidth >= 800;
        final children = [
          connectionCard(stretch: wide),
          cacheCard(stretch: wide),
        ];
        if (bounds.maxWidth < 800) {
          return Column(
            children: [children[0], const SizedBox(height: 16), children[1]],
          );
        }
        return IntrinsicHeight(
          child: Row(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              Expanded(child: children[0]),
              const SizedBox(width: 16),
              Expanded(child: children[1]),
            ],
          ),
        );
      },
    ),
    const SizedBox(height: 16),
    SizedBox(
      width: double.infinity,
      child: card(
        Wrap(
          spacing: 20,
          runSpacing: 8,
          crossAxisAlignment: WrapCrossAlignment.center,
          children: [
            Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                label('保留日志', color: textSecondary, weight: FontWeight.w600),
                const SizedBox(width: 10),
                Switch(
                  value: keepLogs,
                  onChanged: ready && !busy
                      ? (value) => setPreference('keepLogs', value)
                      : null,
                ),
              ],
            ),
            label(keepLogs ? '跨次保留，按需导出' : '下次启动时清理', size: 13),
            TextButton.icon(
              onPressed: ready && !busy && !running ? logSettings : null,
              icon: const Icon(Icons.folder_outlined, size: 19),
              label: const Text('日志目录'),
            ),
            TextButton.icon(
              onPressed: ready && !running && !busy ? exportLogs : null,
              icon: const Icon(Icons.file_download_outlined, size: 19),
              label: const Text('导出日志'),
            ),
          ],
        ),
      ),
    ),
  ];
  Widget metric(String title, String value, IconData icon, Color tint) =>
      Expanded(
        child: card(
          Row(
            children: [
              Container(
                padding: const EdgeInsets.all(12),
                decoration: BoxDecoration(
                  color: Theme.of(context).colorScheme.surfaceContainerHighest,
                  borderRadius: BorderRadius.circular(13),
                ),
                child: Icon(icon, color: tint, size: 22),
              ),
              const SizedBox(width: 16),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    label(
                      value,
                      color: textPrimary,
                      size: 27,
                      weight: FontWeight.w600,
                    ),
                    const SizedBox(height: 4),
                    label(title, size: 13),
                  ],
                ),
              ),
            ],
          ),
          padding: const EdgeInsets.all(20),
        ),
      );
  Widget titleRow(String title, IconData icon, {Widget? action}) => Row(
    children: [
      Icon(icon, color: muted, size: 19),
      const SizedBox(width: 10),
      label(title, color: textPrimary, size: 16, weight: FontWeight.w600),
      const Spacer(),
      ?action,
    ],
  );
  Widget connectionCard({bool stretch = false}) => card(
    Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        titleRow(
          '连接',
          Icons.route_outlined,
          action: IconButton(
            tooltip: '连接设置',
            onPressed: ready && !running && !busy ? connectionSettings : null,
            icon: const Icon(Icons.tune_rounded, size: 19),
          ),
        ),
        const SizedBox(height: 8),
        label('本应用', color: textSecondary, weight: FontWeight.w600),
        const SizedBox(height: 8),
        label('监听端口 ${port.text}'),
        const SizedBox(height: 5),
        label('混合接入 · HTTP / HTTPS / SOCKS4 / SOCKS5', size: 13),
        Row(
          children: [
            Expanded(
              child: label(
                '局域网连接',
                color: textSecondary,
                weight: FontWeight.w600,
              ),
            ),
            Switch(
              value: lan,
              onChanged: ready && !busy ? (v) => unawaited(setLan(v)) : null,
            ),
          ],
        ),
        const Divider(height: 16),
        Row(
          children: [
            Expanded(
              child: label(
                '上游代理',
                color: textSecondary,
                weight: FontWeight.w600,
              ),
            ),
            Switch(
              value: proxy,
              onChanged: ready && !running && !busy
                  ? (value) => setPreference('proxy', value)
                  : null,
            ),
          ],
        ),
        if (proxy)
          label('$protocol · ${host.text}:${proxyPort.text}', size: 14),
        if (stretch) const Spacer(),
        const SizedBox(height: 8),
        const Divider(height: 1),
        const SizedBox(height: 13),
        Row(
          children: [
            Icon(Icons.verified_user_outlined, color: muted, size: 17),
            const SizedBox(width: 8),
            label('CA 证书', size: 14),
            const Spacer(),
            TextButton(
              onPressed: ready && !busy ? caSettings : null,
              child: const Text('管理证书'),
            ),
          ],
        ),
      ],
    ),
  );
  String bytes(String key) {
    final value = int.tryParse(stats[key] ?? '') ?? 0;
    return value >= 1073741824
        ? '${(value / 1073741824).toStringAsFixed(2)} GiB'
        : '${(value / 1048576).toStringAsFixed(1)} MiB';
  }

  Widget cacheCard({bool stretch = false}) => card(
    Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        titleRow(
          '资源缓存',
          Icons.layers_outlined,
          action: IconButton(
            tooltip: '缓存设置',
            onPressed: ready && !running && !busy ? cacheSettings : null,
            icon: const Icon(Icons.tune_rounded, size: 19),
          ),
        ),
        const SizedBox(height: 15),
        Row(
          crossAxisAlignment: CrossAxisAlignment.end,
          children: [
            label(
              bytes('diskBytes'),
              color: textPrimary,
              size: 30,
              weight: FontWeight.w600,
            ),
            const SizedBox(width: 10),
            Padding(
              padding: const EdgeInsets.only(bottom: 5),
              child: label('磁盘缓存', size: 14),
            ),
          ],
        ),
        const SizedBox(height: 18),
        Row(
          children: [
            label('内存缓存', size: 14),
            const Spacer(),
            label(
              '${bytes('memoryBytes')} / ${memory.text} MiB',
              color: textPrimary,
              size: 14,
            ),
          ],
        ),
        const SizedBox(height: 10),
        ClipRRect(
          borderRadius: BorderRadius.circular(4),
          child: LinearProgressIndicator(
            value:
                ((int.tryParse(stats['memoryBytes'] ?? '') ?? 0) /
                        ((int.tryParse(memory.text) ?? 0) * 1048576 + 1))
                    .clamp(0, 1),
            minHeight: 5,
            backgroundColor: line,
            color: mint,
          ),
        ),
        const SizedBox(height: 12),
        label('内存命中 ${stats['memoryHits'] ?? '0'}', size: 14),
        if (stretch) const Spacer(),
        const SizedBox(height: 22),
        const Divider(height: 1),
        const SizedBox(height: 13),
        Row(
          children: [
            TextButton.icon(
              onPressed: ready && !busy && !running ? cacheSettings : null,
              icon: const Icon(Icons.folder_outlined, size: 18),
              label: const Text('缓存目录'),
            ),
            const Spacer(),
            TextButton(
              onPressed: ready && !running && !busy
                  ? () async {
                      if (await confirm('清理缓存', '删除已下载的资源并归零计数。证书和设置会保留。')) {
                        await perform(() async {
                          await core.call('clear');
                        });
                      }
                    }
                  : null,
              child: const Text('清理缓存'),
            ),
          ],
        ),
      ],
    ),
  );
  Widget field(String title, Widget input) => Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      ExcludeSemantics(
        child: label(
          title,
          color: textSecondary,
          size: 15,
          weight: FontWeight.w500,
        ),
      ),
      const SizedBox(height: 10),
      Semantics(label: title, child: input),
    ],
  );

  Widget focusSave(Future<void> Function() save, Widget child) => Focus(
    onFocusChange: (focused) {
      if (!focused) unawaited(save());
    },
    child: child,
  );

  Future<void> connectionSettings() async {
    bool showPassword = false, testing = false;
    String probeResult = '';
    bool probeOk = false;
    final draft = {...settings(), 'password': password.text};
    bool saving = false;
    String failure = '';
    final proxy = this.proxy;
    String protocol = this.protocol;
    Map<String, String> draftSettings() => {
      ...settings(),
      for (final entry in draft.entries)
        entry.key: entry.key == 'password' || entry.key == 'username'
            ? entry.value
            : entry.value.trim(),
      'proxy': '$proxy',
      'protocol': protocol,
    };
    Map<String, String>? lastSaved = draftSettings();
    Future<void>? pending;
    Future<void> save(BuildContext c, StateSetter update) async {
      if (pending != null) {
        await pending;
        if (!c.mounted) return;
        return save(c, update);
      }
      if (!c.mounted) return;
      final operation = () async {
        final listen = int.tryParse(draft['port']!),
            upstream = int.tryParse(draft['proxyPort']!);
        if (listen == null ||
            listen < 1 ||
            listen > 65535 ||
            (proxy &&
                (upstream == null ||
                    upstream < 1 ||
                    upstream > 65535 ||
                    draft['host']!.trim().isEmpty))) {
          update(() => failure = '请填写有效地址和 1–65535 的端口');
          return;
        }
        if (proxy &&
            protocol != 'SOCKS4' &&
            draft['username']!.isEmpty &&
            draft['password']!.isNotEmpty) {
          update(() => failure = '请填写上游代理用户名');
          return;
        }
        final candidate = draftSettings();
        if (lastSaved != null && mapEquals(candidate, lastSaved)) {
          update(() => failure = '');
          return;
        }
        update(() {
          saving = true;
          failure = '';
        });
        try {
          final args = draftSettings();
          final changed = Map<String, String>.of(args);
          if (passwordUnavailable && draft['password'] == password.text) {
            changed.remove('password');
          }
          await core.call('settings', changed);
          lastSaved = Map.of(args);
          if (mounted) {
            setState(() {
              port.text = args['port']!;
              host.text = args['host']!;
              proxyPort.text = args['proxyPort']!;
              username.text = args['username']!;
              password.text = args['password']!;
              if (changed.containsKey('password')) passwordUnavailable = false;
              this.protocol = protocol;
              this.proxy = proxy;
            });
          }
          if (c.mounted) {
            update(() => saving = false);
          }
        } catch (e) {
          if (c.mounted) {
            update(() {
              saving = false;
              failure = e.toString().replaceFirst('Bad state: ', '');
            });
          }
        }
      }();
      pending = operation;
      try {
        await operation;
      } finally {
        if (identical(pending, operation)) pending = null;
      }
    }

    await showDialog<void>(
      barrierDismissible: false,
      context: context,
      builder: (c) => StatefulBuilder(
        builder: (c, update) => PopScope(
          canPop: false,
          onPopInvokedWithResult: (didPop, result) async {
            if (didPop) return;
            await save(c, update);
            if (c.mounted && failure.isEmpty && ModalRoute.of(c)!.isCurrent) {
              Navigator.pop(c);
            }
          },
          child: AlertDialog(
            titlePadding: const EdgeInsets.fromLTRB(28, 24, 28, 24),
            contentPadding: const EdgeInsets.fromLTRB(28, 0, 28, 8),
            actionsPadding: const EdgeInsets.fromLTRB(20, 16, 24, 24),
            title: Row(
              children: [
                Expanded(child: Text('连接设置')),
                IconButton(
                  tooltip: '关闭设置',
                  onPressed: () async {
                    await save(c, update);
                    if (c.mounted &&
                        failure.isEmpty &&
                        ModalRoute.of(c)!.isCurrent) {
                      Navigator.pop(c);
                    }
                  },
                  icon: const Icon(Icons.close),
                ),
              ],
            ),
            content: SizedBox(
              width: 560,
              child: SingleChildScrollView(
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    label(
                      '本应用',
                      color: textSecondary,
                      size: 15,
                      weight: FontWeight.w600,
                    ),
                    const SizedBox(height: 18),
                    field(
                      '监听端口',
                      focusSave(
                        () => save(c, update),
                        TextFormField(
                          onChanged: (value) => update(() {
                            draft['port'] = value;
                            probeResult = '';
                          }),
                          initialValue: draft['port'],
                          enabled: !testing && !saving,
                          keyboardType: TextInputType.number,
                        ),
                      ),
                    ),
                    const SizedBox(height: 10),
                    label('混合接入 · HTTP / HTTPS / SOCKS4 / SOCKS5'),
                    const Divider(height: 40),
                    label(
                      '上游代理',
                      color: textSecondary,
                      size: 15,
                      weight: FontWeight.w600,
                    ),
                    const SizedBox(height: 12),
                    Row(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Expanded(
                          flex: 2,
                          child: field(
                            '协议',
                            ProtocolSelector(
                              value: protocol,
                              onChanged: !testing && !saving
                                  ? (p) async {
                                      update(() {
                                        protocol = p;
                                        probeResult = '';
                                      });
                                      await save(c, update);
                                    }
                                  : null,
                            ),
                          ),
                        ),
                        const SizedBox(width: 20),
                        Expanded(
                          flex: 4,
                          child: field(
                            '地址',
                            focusSave(
                              () => save(c, update),
                              TextFormField(
                                onChanged: (value) => update(() {
                                  draft['host'] = value;
                                  probeResult = '';
                                }),
                                initialValue: draft['host'],
                                enabled: !testing && !saving,
                              ),
                            ),
                          ),
                        ),
                        const SizedBox(width: 20),
                        Expanded(
                          flex: 2,
                          child: field(
                            '端口',
                            focusSave(
                              () => save(c, update),
                              TextFormField(
                                onChanged: (value) => update(() {
                                  draft['proxyPort'] = value;
                                  probeResult = '';
                                }),
                                initialValue: draft['proxyPort'],
                                enabled: !testing && !saving,
                                keyboardType: TextInputType.number,
                              ),
                            ),
                          ),
                        ),
                      ],
                    ),
                    const SizedBox(height: 22),
                    Row(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Expanded(
                          child: field(
                            '用户名（选填）',
                            focusSave(
                              () => save(c, update),
                              TextFormField(
                                onChanged: (value) => update(() {
                                  draft['username'] = value;
                                  probeResult = '';
                                }),
                                initialValue: draft['username'],
                                enabled: !testing && !saving,
                              ),
                            ),
                          ),
                        ),
                        const SizedBox(width: 20),
                        Expanded(
                          child: field(
                            '密码（选填）',
                            focusSave(
                              () => save(c, update),
                              TextFormField(
                                onChanged: (value) => update(() {
                                  draft['password'] = value;
                                  probeResult = '';
                                }),
                                initialValue: draft['password'],
                                obscureText: !showPassword,
                                enabled:
                                    !testing && !saving && protocol != 'SOCKS4',
                                decoration: InputDecoration(
                                  suffixIcon: IconButton(
                                    tooltip: showPassword ? '隐藏密码' : '显示密码',
                                    onPressed:
                                        !testing &&
                                            !saving &&
                                            protocol != 'SOCKS4'
                                        ? () => update(
                                            () => showPassword = !showPassword,
                                          )
                                        : null,
                                    icon: Icon(
                                      showPassword
                                          ? Icons.visibility_off_outlined
                                          : Icons.visibility_outlined,
                                      size: 19,
                                    ),
                                  ),
                                ),
                              ),
                            ),
                          ),
                        ),
                      ],
                    ),
                    const Divider(height: 32),
                    Row(
                      children: [
                        Expanded(child: label('连通性检测', color: textSecondary)),
                        OutlinedButton(
                          onPressed: testing || saving
                              ? null
                              : () async {
                                  await save(c, update);
                                  if (!c.mounted || failure.isNotEmpty) return;
                                  final args = draftSettings();
                                  update(() {
                                    testing = true;
                                    probeResult = '';
                                  });
                                  try {
                                    await core.call('probe', args);
                                    if (c.mounted) {
                                      update(() {
                                        probeOk = true;
                                        probeResult = '当前配置可以访问游戏站点';
                                      });
                                    }
                                  } catch (e) {
                                    if (c.mounted) {
                                      update(() {
                                        probeOk = false;
                                        probeResult = e.toString().replaceFirst(
                                          'Bad state: ',
                                          '',
                                        );
                                      });
                                    }
                                  } finally {
                                    if (c.mounted) {
                                      update(() => testing = false);
                                    }
                                  }
                                },
                          child: Text(testing ? '检测中…' : '检测连接'),
                        ),
                      ],
                    ),
                    if (failure.isNotEmpty)
                      Text(failure, style: TextStyle(color: errorColor)),
                    if (probeResult.isNotEmpty)
                      Padding(
                        padding: const EdgeInsets.only(top: 14),
                        child: Text(
                          probeResult,
                          style: TextStyle(color: probeOk ? mint : errorColor),
                        ),
                      ),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }

  Future<void> cacheSettings() async {
    String draft = memory.text;
    String? selected;
    bool migrate = false, saving = false;
    String failure = '';
    Future<void>? pending;
    Future<void> save(BuildContext c, StateSetter update) async {
      if (pending != null) {
        await pending;
        if (!c.mounted) return;
        return save(c, update);
      }
      if (!c.mounted) return;
      final operation = () async {
        final value = int.tryParse(draft);
        if (value == null || value < 0 || value > 8796093022207) {
          update(() => failure = '请输入有效的非负整数');
          return;
        }
        if (draft.trim() == memory.text && selected == null) {
          update(() => failure = '');
          return;
        }
        update(() {
          saving = true;
          failure = '';
        });
        setState(() => busy = true);
        try {
          final args = {'memoryMiB': draft.trim()};
          Map<String, String>? result;
          if (selected != null) {
            result = await core.call('directory', {
              ...args,
              'kind': 'cache',
              'path': selected!,
              'migrate': '$migrate',
            });
          } else {
            await core.call('settings', args);
          }
          if (mounted) {
            setState(() {
              memory.text = draft.trim();
              if (result != null) cachePath = result['path']!;
              selected = null;
            });
          }
          if (c.mounted) {
            update(() => saving = false);
          }
          await refresh();
        } catch (e) {
          if (c.mounted) {
            update(() {
              saving = false;
              failure = e.toString().replaceFirst('Bad state: ', '');
            });
          }
        } finally {
          if (mounted) setState(() => busy = false);
        }
      }();
      pending = operation;
      try {
        await operation;
      } finally {
        if (identical(pending, operation)) pending = null;
      }
    }

    await showDialog<void>(
      barrierDismissible: false,
      context: context,
      builder: (c) => StatefulBuilder(
        builder: (c, update) => PopScope(
          canPop: false,
          onPopInvokedWithResult: (didPop, result) async {
            if (didPop) return;
            await save(c, update);
            if (c.mounted && failure.isEmpty && ModalRoute.of(c)!.isCurrent) {
              Navigator.pop(c);
            }
          },
          child: AlertDialog(
            titlePadding: const EdgeInsets.fromLTRB(28, 24, 28, 24),
            contentPadding: const EdgeInsets.fromLTRB(28, 0, 28, 8),
            actionsPadding: const EdgeInsets.fromLTRB(20, 16, 24, 24),
            title: Row(
              children: [
                Expanded(child: Text('缓存设置')),
                IconButton(
                  tooltip: '关闭设置',
                  onPressed: () async {
                    await save(c, update);
                    if (c.mounted &&
                        failure.isEmpty &&
                        ModalRoute.of(c)!.isCurrent) {
                      Navigator.pop(c);
                    }
                  },
                  icon: const Icon(Icons.close),
                ),
              ],
            ),
            content: SizedBox(
              width: 470,
              child: SingleChildScrollView(
                child: Column(
                  mainAxisSize: MainAxisSize.min,
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    field(
                      '内存上限',
                      focusSave(
                        () => save(c, update),
                        TextFormField(
                          initialValue: draft,
                          onChanged: (value) => draft = value,
                          enabled: !saving,
                          keyboardType: TextInputType.number,
                          decoration: const InputDecoration(suffixText: 'MiB'),
                        ),
                      ),
                    ),
                    const SizedBox(height: 12),
                    label('0 为关闭内存缓存。磁盘缓存不设容量上限。'),
                    const Divider(height: 40),
                    label(
                      '缓存目录',
                      color: textSecondary,
                      weight: FontWeight.w600,
                    ),
                    const SizedBox(height: 14),
                    SelectableText(
                      displayPath(
                        selected == null
                            ? cachePath
                            : p.join(selected!, 'gbf-flash-cache-cache'),
                      ),
                      style: TextStyle(color: muted, fontSize: 14),
                    ),
                    CheckboxListTile(
                      contentPadding: EdgeInsets.zero,
                      value: migrate,
                      onChanged: saving
                          ? null
                          : (v) => update(() => migrate = v!),
                      title: const Text('更换目录时迁移已有缓存'),
                      subtitle: const Text('复制资源，旧目录保留'),
                    ),
                    TextButton.icon(
                      onPressed: saving
                          ? null
                          : () async {
                              final folder = await getDirectoryPath(
                                confirmButtonText: '选择目录',
                              );
                              if (folder != null && c.mounted) {
                                update(() => selected = folder);
                                await save(c, update);
                              }
                            },
                      icon: const Icon(Icons.folder_open, size: 19),
                      label: const Text('选择目录'),
                    ),
                    if (failure.isNotEmpty)
                      Text(failure, style: TextStyle(color: errorColor)),
                  ],
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }

  Future<void> logSettings() async {
    String? selected;
    bool saving = false;
    String failure = '';
    await showDialog<void>(
      barrierDismissible: false,
      context: context,
      builder: (c) => StatefulBuilder(
        builder: (c, update) => PopScope(
          canPop: !saving,
          child: AlertDialog(
            titlePadding: const EdgeInsets.fromLTRB(28, 24, 28, 24),
            contentPadding: const EdgeInsets.fromLTRB(28, 0, 28, 8),
            actionsPadding: const EdgeInsets.fromLTRB(20, 16, 24, 24),
            title: const Text('日志目录'),
            content: SizedBox(
              width: 470,
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  SelectableText(
                    displayPath(
                      selected == null
                          ? logsPath
                          : p.join(selected!, 'gbf-flash-cache-logs'),
                    ),
                  ),
                  const SizedBox(height: 16),
                  TextButton.icon(
                    onPressed: saving
                        ? null
                        : () async {
                            final folder = await getDirectoryPath(
                              confirmButtonText: '选择目录',
                            );
                            if (folder != null && c.mounted) {
                              update(() => selected = folder);
                            }
                          },
                    icon: const Icon(Icons.folder_open, size: 19),
                    label: const Text('选择目录'),
                  ),
                  if (failure.isNotEmpty)
                    Text(failure, style: TextStyle(color: errorColor)),
                ],
              ),
            ),
            actions: [
              TextButton(
                onPressed: saving ? null : () => Navigator.pop(c),
                child: const Text('取消'),
              ),
              FilledButton(
                onPressed: saving
                    ? null
                    : () async {
                        if (selected == null) {
                          Navigator.pop(c);
                          return;
                        }
                        update(() => saving = true);
                        setState(() => busy = true);
                        try {
                          final result = await core.call('directory', {
                            'kind': 'logs',
                            'path': selected!,
                          });
                          if (mounted) {
                            setState(() => logsPath = result['path']!);
                          }
                          if (c.mounted) {
                            update(() => saving = false);
                            Navigator.pop(c);
                          }
                        } catch (e) {
                          if (c.mounted) {
                            update(() {
                              saving = false;
                              failure = e.toString().replaceFirst(
                                'Bad state: ',
                                '',
                              );
                            });
                          }
                        } finally {
                          if (mounted) setState(() => busy = false);
                        }
                      },
                child: Text(saving ? '保存中' : '保存'),
              ),
            ],
          ),
        ),
      ),
    );
  }

  Widget helpPage() => const HelpGuide(mobile: false);
  void about() => showDialog<void>(
    context: context,
    builder: (c) => AlertDialog(
      titlePadding: const EdgeInsets.fromLTRB(28, 24, 28, 24),
      contentPadding: const EdgeInsets.fromLTRB(28, 0, 28, 8),
      actionsPadding: const EdgeInsets.fromLTRB(20, 16, 24, 24),
      title: const Text('GBF Flash Cache'),
      content: SizedBox(
        width: 420,
        child: SingleChildScrollView(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            mainAxisSize: MainAxisSize.min,
            children: [
              Text('版本 $appVersion', style: TextStyle(color: mint)),
              SizedBox(height: 10),
              Text(
                'Copyright © 2026 ErinnerMO\nMIT License',
                style: TextStyle(color: muted, height: 1.6),
              ),
              SizedBox(height: 24),
              Text(
                '非官方工具，与游戏运营方无隶属关系。请遵守相关服务条款。\n\n指定域名的 HTTPS 流量在本机处理，请保护 CA 私钥。软件按现状提供，不保证提速、持续可用或账号不受限制；责任范围以随包许可证和适用法律为准。\n\n完整许可证与第三方声明随程序提供。',
                style: TextStyle(color: muted, height: 1.7),
              ),
            ],
          ),
        ),
      ),
      actions: [
        TextButton(
          onPressed: () async {
            try {
              final text = await File(WindowsHost.licensePath).readAsString();
              if (!mounted || !c.mounted) return;
              await showDialog<void>(
                context: c,
                builder: (dialog) => AlertDialog(
                  titlePadding: const EdgeInsets.fromLTRB(28, 24, 28, 24),
                  contentPadding: const EdgeInsets.fromLTRB(28, 0, 28, 8),
                  actionsPadding: const EdgeInsets.fromLTRB(20, 16, 24, 24),
                  title: const Text('MIT License'),
                  content: SizedBox(
                    width: 600,
                    child: SingleChildScrollView(child: SelectableText(text)),
                  ),
                  actions: [
                    TextButton(
                      onPressed: () => Navigator.pop(dialog),
                      child: const Text('关闭'),
                    ),
                  ],
                ),
              );
            } catch (_) {
              toast('无法读取随包许可证文件');
            }
          },
          child: const Text('MIT 许可证'),
        ),

        TextButton(onPressed: () => Navigator.pop(c), child: const Text('关闭')),
      ],
    ),
  );
}
