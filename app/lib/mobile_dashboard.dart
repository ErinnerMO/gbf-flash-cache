import 'protocol_selector.dart';
import 'help_guide.dart';

import 'dart:async';

import 'mobile_proxy.dart';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

class MobileDashboard extends StatefulWidget {
  const MobileDashboard({super.key, this.onThemeChanged});
  final ValueChanged<ThemeMode>? onThemeChanged;
  @override
  State<MobileDashboard> createState() => _MobileDashboardState();
}

class _MobileDashboardState extends State<MobileDashboard>
    with WidgetsBindingObserver {
  static const channel = MethodChannel('gbf/core');
  final fields = {
    'port': TextEditingController(text: '8765'),
    'host': TextEditingController(text: '127.0.0.1'),
    'proxyPort': TextEditingController(text: '7890'),
    'username': TextEditingController(),
    'password': TextEditingController(),
    'memoryMiB': TextEditingController(text: '128'),
  };
  Map<String, String> saved = {}, stats = {}, ca = {};
  String page = '', error = '', protocol = 'HTTP', directoryWarning = '';
  ThemeMode themeMode = ThemeMode.dark;
  bool get dark => Theme.of(context).brightness == Brightness.dark;
  bool passwordUnavailable = false;
  String get visibleError => [
    error,
    statusWarning,
    directoryWarning,
  ].where((s) => s.isNotEmpty).join('\n');
  String statusWarning = '';
  Future<Map<String, String>>? statusRequest;
  String probeState = '', probeError = '';
  bool systemProxyEnabled = false;
  Future<void>? pendingSave;
  bool proxy = false, lan = false, keepLogs = false;
  bool ready = false, running = false, busy = false, polling = false;
  Timer? timer;
  bool get editable => ready && !running && !busy;
  Future<Map<String, String>> call(
    String op, [
    Map<String, String> args = const {},
  ]) async => Map<String, String>.from(
    await channel.invokeMapMethod<String, String>(op, args) ?? {},
  );
  Map<String, String> settings() => {
    for (final f in fields.entries)
      if (f.key != 'password' || !passwordUnavailable)
        f.key: f.key == 'password' || f.key == 'username'
            ? f.value.text
            : f.value.text.trim(),
    'proxy': '$proxy',
    'protocol': protocol,
    'lan': '$lan',
    'keepLogs': '$keepLogs',
  };
  void restore() {
    passwordUnavailable = saved.containsKey('passwordWarning');
    for (final f in fields.entries) {
      f.value.text =
          saved[f.key] ??
          switch (f.key) {
            'port' => '8765',
            'host' => '127.0.0.1',
            'proxyPort' => '7890',
            'memoryMiB' => '128',
            _ => '',
          };
    }
    systemProxyEnabled = saved['systemProxyEnabled'] == 'true';
    proxy = saved['proxy'] == 'true';
    protocol = saved['protocol'] ?? 'HTTP';
    lan = saved['lan'] == 'true';
    keepLogs = saved['keepLogs'] == 'true';
  }

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    unawaited(initialize());
  }

  Future<void> initialize() async {
    try {
      final result = await call('init');
      if (!mounted) return;
      setState(() {
        saved = result;
        error = result['passwordWarning'] ?? result['startupWarning'] ?? '';
        directoryWarning = result['directoryWarning'] ?? '';
        restore();
        ready = true;
      });
      themeMode = switch (saved['themeMode']) {
        'light' => ThemeMode.light,
        'system' => ThemeMode.system,
        _ => ThemeMode.dark,
      };
      widget.onThemeChanged?.call(themeMode);
      timer = Timer.periodic(const Duration(seconds: 3), (_) {
        unawaited(refresh());
      });
      await refresh();
    } catch (e) {
      if (mounted) setState(() => error = message(e));
    }
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed) unawaited(refresh());
  }

  String message(Object e) =>
      e is PlatformException ? (e.message ?? e.code) : e.toString();
  Future<void> refresh() async {
    if (!mounted || polling || !ready) return;
    polling = true;
    try {
      // Keep one native request outstanding: timeouts must not flood its serial queue.
      statusRequest ??= call('status');
      final result = await statusRequest!.timeout(const Duration(seconds: 10));
      statusRequest = null;
      if (result['running'] != 'true' && result['running'] != 'false') {
        throw const FormatException('服务状态响应不完整');
      }
      if (mounted) {
        setState(() {
          stats = result;
          statusWarning = '';
          directoryWarning = result['directoryWarning'] ?? '';
          running = result['running'] == 'true';
          if ((result['proxyWarning'] ?? '').isNotEmpty) {
            error = result['proxyWarning']!;
          }
        });
      }
    } on TimeoutException {
      if (mounted) setState(() => statusWarning = '统计刷新超时，等待服务响应');
    } catch (e) {
      statusRequest = null;
      if (mounted) setState(() => statusWarning = '统计刷新失败：${message(e)}');
    } finally {
      polling = false;
    }
  }

  Future<void> perform(
    Future<void> Function() operation, {
    bool refreshStatus = true,
  }) async {
    if (busy || !ready) return;
    setState(() {
      busy = true;
      error = '';
    });
    try {
      await operation();
      if (refreshStatus) await refresh();
    } catch (e) {
      if (mounted && !(e is PlatformException && e.code == 'cancelled')) {
        setState(() => error = message(e));
      }
    } finally {
      if (mounted) setState(() => busy = false);
    }
  }

  void toast(String text) {
    if (mounted) {
      ScaffoldMessenger.of(context).showSnackBar(SnackBar(content: Text(text)));
    }
  }

  Future<bool> confirm(String title, String text) async =>
      await showDialog<bool>(
        context: context,
        builder: (c) => AlertDialog(
          title: Text(title),
          content: Text(text),
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
  void validate({bool? upstream}) {
    final useProxy = upstream ?? proxy;
    for (final key in ['port', if (useProxy) 'proxyPort']) {
      final n = int.tryParse(fields[key]!.text.trim());
      if (n == null || n < 1 || n > 65535) {
        throw const FormatException('端口须为 1–65535');
      }
    }
    final memory = int.tryParse(fields['memoryMiB']!.text.trim());
    if (memory == null || memory < 0 || memory > 8796093022207) {
      throw const FormatException('请输入有效的非负整数');
    }
    if (useProxy && fields['host']!.text.trim().isEmpty) {
      throw const FormatException('请填写上游代理地址');
    }
    if (useProxy &&
        protocol != 'SOCKS4' &&
        fields['username']!.text.isEmpty &&
        fields['password']!.text.isNotEmpty) {
      throw const FormatException('请填写上游代理用户名');
    }
  }

  bool get editing => const ['上游代理设置', '监听端口', '缓存设置'].contains(page);

  Future<void> save() async {
    if (pendingSave != null) {
      await pendingSave;
      return save();
    }
    if (!editable || !editing) return;
    final connection = page == '上游代理设置';
    final args = connection
        ? (settings()
            ..remove('memoryMiB')
            ..remove('port'))
        : page == '监听端口'
        ? {'port': fields['port']!.text.trim()}
        : {'memoryMiB': fields['memoryMiB']!.text.trim()};
    if (args.entries.every((e) => saved[e.key] == e.value)) {
      if (mounted && error.isNotEmpty) setState(() => error = '');
      return;
    }
    final operation = () async {
      if (mounted) setState(() => error = '');
      try {
        if (connection) {
          validate();
        } else if (page == '监听端口') {
          final n = int.tryParse(args['port']!);
          if (n == null || n < 1 || n > 65535) {
            throw const FormatException('端口须为 1–65535');
          }
        } else {
          final n = int.tryParse(args['memoryMiB']!);
          if (n == null || n < 0 || n > 8796093022207) {
            throw const FormatException('请输入有效的非负整数');
          }
        }
        await call('settings', args);
        saved = {...saved, ...args};
        if (connection && args.containsKey('password')) {
          saved.remove('passwordWarning');
          passwordUnavailable = false;
        }
      } catch (e) {
        if (mounted) setState(() => error = message(e));
      }
    }();
    pendingSave = operation;
    try {
      await operation;
    } finally {
      pendingSave = null;
    }
  }

  Future<void> startConfigured(Map<String, String> args) async {
    await call('start', args);
    if (systemProxyEnabled) {
      try {
        await call('proxy_start');
      } catch (_) {
        await call('stop');
        await refresh();
        rethrow;
      }
    }
  }

  void open(String value) {
    if (!ready) return;
    setState(() {
      restore();
      error = '';
      page = value;
    });
    if (value == 'CA 证书') {
      unawaited(loadCertificate());
    }
  }

  Future<void> loadCertificate() async {
    try {
      final result = await call('ca_status');
      if (mounted) setState(() => ca = result);
    } catch (e) {
      if (mounted && page == 'CA 证书') setState(() => error = message(e));
    }
  }

  Future<void> back() async {
    await save();
    if (!mounted || busy || (editing && error.isNotEmpty)) {
      return;
    }
    FocusManager.instance.primaryFocus?.unfocus();
    setState(() {
      restore();
      error = '';
      page = page == '使用说明' ? '设置' : '';
    });
  }

  String size(String key) {
    final bytes = int.tryParse(stats[key] ?? '') ?? 0;
    if (bytes < 1024) return '$bytes B';
    if (bytes < 1048576) return '${(bytes / 1024).toStringAsFixed(1)} KiB';
    return '${(bytes / 1048576).toStringAsFixed(1)} MiB';
  }

  double get memoryFraction {
    final maximum = int.tryParse(saved['memoryMiB'] ?? '128') ?? 128;
    return maximum == 0
        ? 0
        : ((int.tryParse(stats['memoryBytes'] ?? '') ?? 0) /
                  (maximum * 1048576))
              .clamp(0, 1);
  }

  Widget heading(String title) =>
      Text(title, style: Theme.of(context).textTheme.titleLarge);
  Widget card(List<Widget> children) => Card(
    margin: const EdgeInsets.only(bottom: 16),
    shape: RoundedRectangleBorder(
      borderRadius: BorderRadius.circular(16),
      side: BorderSide(color: Theme.of(context).dividerColor),
    ),
    child: Padding(
      padding: const EdgeInsets.all(16),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: children,
      ),
    ),
  );
  Widget row(String title, Widget trailing, {VoidCallback? onTap}) => ListTile(
    minTileHeight: 48,
    title: Text(title),
    trailing: trailing,
    onTap: onTap,
  );
  Widget entry(String title, {VoidCallback? onTap}) => ListTile(
    minTileHeight: 48,
    title: Text(title),
    trailing: const Icon(Icons.chevron_right),
    onTap: !ready || busy ? null : onTap ?? () => open(title),
  );
  Widget field(String key, String label, {bool numeric = false}) => Padding(
    padding: const EdgeInsets.only(top: 18),
    child: Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(label),
        const SizedBox(height: 8),
        Focus(
          onFocusChange: (focused) {
            if (!focused && !busy) unawaited(save());
          },
          child: TextField(
            onSubmitted: (_) => unawaited(save()),
            onTapOutside: (_) => FocusManager.instance.primaryFocus?.unfocus(),
            key: ValueKey(key),
            controller: fields[key],
            onChanged: (_) => setState(() {
              if (key == 'password') passwordUnavailable = false;
              if (key != 'memoryMiB') clearProbe();
            }),
            enabled: editable,
            obscureText: key == 'password',
            autocorrect: false,
            enableSuggestions: key != 'password',
            keyboardType: numeric ? TextInputType.number : TextInputType.text,
            decoration: InputDecoration(
              suffixText: key == 'memoryMiB' ? 'MiB' : null,
            ),
          ),
        ),
      ],
    ),
  );
  void clearProbe() {
    probeState = '';
    probeError = '';
  }

  Widget probeControl() {
    final state = probeState;
    final failed = state == '连接失败';
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        row(
          '连通性检测',
          state == '检测中'
              ? const SizedBox(
                  width: 18,
                  height: 18,
                  child: CircularProgressIndicator(strokeWidth: 2),
                )
              : state.isEmpty
              ? const Icon(Icons.chevron_right)
              : Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    Icon(
                      failed ? Icons.error_outline : Icons.check_circle_outline,
                      size: 18,
                      color: failed
                          ? Theme.of(context).colorScheme.error
                          : Theme.of(context).colorScheme.primary,
                    ),
                    const SizedBox(width: 6),
                    Text(
                      state,
                      style: TextStyle(
                        color: failed
                            ? Theme.of(context).colorScheme.error
                            : Theme.of(context).colorScheme.primary,
                      ),
                    ),
                  ],
                ),
          onTap: !ready || busy
              ? null
              : () => perform(() async {
                  setState(() {
                    probeState = '检测中';
                    probeError = '';
                  });
                  try {
                    validate();
                    await call('probe', settings());
                    if (mounted) setState(() => probeState = '连接正常');
                  } catch (e) {
                    if (mounted) {
                      setState(() {
                        probeState = '连接失败';
                        probeError = message(e);
                      });
                    }
                  }
                }, refreshStatus: false),
        ),
        if (failed)
          Padding(
            padding: const EdgeInsets.only(bottom: 12),
            child: Text(
              probeError,
              style: TextStyle(color: Theme.of(context).colorScheme.error),
            ),
          ),
      ],
    );
  }

  Widget get stoppedHint => running
      ? const Padding(
          padding: EdgeInsets.only(bottom: 16),
          child: Text('停止服务后可修改配置。'),
        )
      : const SizedBox.shrink();
  List<Widget> home() => [
    card([
      Row(
        children: [
          Icon(
            Icons.power_settings_new,
            size: 36,
            color: running ? Theme.of(context).colorScheme.primary : null,
          ),
          const SizedBox(width: 14),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  !ready
                      ? '正在初始化…'
                      : running
                      ? '缓存服务运行中'
                      : '缓存服务未启动',
                  style: Theme.of(context).textTheme.titleMedium,
                ),
                const SizedBox(height: 6),
                Text('监听端口 ${saved['port'] ?? '8765'}'),
              ],
            ),
          ),
          FilledButton(
            onPressed: !ready || busy
                ? null
                : () => perform(() async {
                    validate();
                    final args = settings();
                    if (running) {
                      await call('stop');
                    } else {
                      await startConfigured(args);
                    }
                    saved = {...saved, ...args};
                  }),
            style: running
                ? FilledButton.styleFrom(
                    backgroundColor: dark
                        ? const Color(0xFF2B2B2B)
                        : const Color(0xFFE5E5E5),
                    foregroundColor: Theme.of(context).colorScheme.onSurface,
                  )
                : null,
            child: Text(running ? '停止' : '启动'),
          ),
        ],
      ),
    ]),
    for (final pair in [
      [('preloaded', '预载完成'), ('requests', '资源回源')],
      [('hits', '缓存命中'), ('failures', '网络异常')],
    ])
      Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          for (var i = 0; i < pair.length; i++) ...[
            if (i != 0) const SizedBox(width: 12),
            Expanded(
              child: card([
                Text(pair[i].$2),
                const SizedBox(height: 8),
                Text(
                  stats[pair[i].$1] ?? '—',
                  style: Theme.of(context).textTheme.headlineMedium?.copyWith(
                    fontWeight: FontWeight.w600,
                    color:
                        pair[i].$1 == 'failures' &&
                            (int.tryParse(stats['failures'] ?? '') ?? 0) > 0
                        ? Theme.of(context).colorScheme.error
                        : null,
                  ),
                ),
              ]),
            ),
          ],
        ],
      ),
    card([
      heading('连接'),
      row(
        '监听端口',
        Row(
          mainAxisSize: MainAxisSize.min,
          children: [
            Text(saved['port'] ?? '8765'),
            const Icon(Icons.chevron_right),
          ],
        ),
        onTap: !ready || busy ? null : () => open('监听端口'),
      ),
      SwitchListTile(
        title: const Text('局域网连接'),
        value: lan,
        onChanged: !ready || busy
            ? null
            : (value) => perform(() async {
                if (running && !await confirm('更改局域网连接', '需要重启缓存服务，当前连接会中断。')) {
                  return;
                }
                final wasRunning = running;
                final args = {...settings(), 'lan': '$value'};
                if (wasRunning) {
                  await call('stop');
                  try {
                    // start saves the settings only after the service starts successfully.
                    await startConfigured(args);
                  } catch (_) {
                    await refresh();
                    rethrow;
                  }
                } else {
                  await call('settings', {'lan': '$value'});
                }
                saved = {...saved, ...args};
                if (mounted) {
                  setState(() {
                    lan = value;
                    clearProbe();
                  });
                }
              }),
      ),
      SwitchListTile(
        title: const Text('上游代理'),
        value: proxy,
        onChanged: !editable
            ? null
            : (value) => perform(() async {
                try {
                  if (value) validate(upstream: true);
                  final args = value
                      ? (settings()
                          ..remove('memoryMiB')
                          ..remove('port')
                          ..remove('lan')
                          ..remove('keepLogs'))
                      : <String, String>{};
                  args['proxy'] = '$value';
                  await call('settings', args);
                  saved = {...saved, ...args};
                  if (mounted) {
                    setState(() {
                      proxy = value;
                      clearProbe();
                    });
                  }
                } catch (e) {
                  if (value && mounted) {
                    open('上游代理设置');
                    setState(() {
                      proxy = true;
                      error = message(e);
                    });
                  } else {
                    rethrow;
                  }
                }
              }, refreshStatus: false),
      ),
      entry('上游代理设置'),
      const Divider(height: 28),
      SwitchListTile(
        title: const Text('系统代理'),
        value: systemProxyEnabled,
        onChanged: !editable
            ? null
            : (value) => perform(() async {
                await call('proxy_save', {'enabled': '$value'});
                saved['systemProxyEnabled'] = '$value';
                if (mounted) {
                  setState(() {
                    systemProxyEnabled = value;
                    clearProbe();
                  });
                }
              }, refreshStatus: false),
      ),
      entry(
        '系统代理设置',
        onTap: !ready || busy
            ? null
            : () async {
                await Navigator.of(context).push(
                  MaterialPageRoute<void>(
                    builder: (_) => MobileProxySettings(active: running),
                  ),
                );
                await refresh();
              },
      ),
    ]),
    card([
      heading('资源缓存'),
      row('磁盘缓存', Text(size('diskBytes'))),
      row(
        '内存缓存',
        Text('${size('memoryBytes')} / ${saved['memoryMiB'] ?? '128'} MiB'),
      ),
      LinearProgressIndicator(
        value: memoryFraction,
        backgroundColor: Theme.of(context).dividerColor,
        borderRadius: BorderRadius.circular(6),
      ),
      const SizedBox(height: 8),
      entry('缓存设置'),
    ]),
    card([entry('CA 证书'), const Divider(), probeControl()]),
  ];
  List<Widget> portPage() => [
    stoppedHint,
    card([
      field('port', '监听端口', numeric: true),
      const SizedBox(height: 10),
      const Text('支持 HTTP、HTTPS、SOCKS4、SOCKS5'),
    ]),
  ];
  List<Widget> connectionPage() => [
    stoppedHint,
    card([
      const Text('协议'),
      const SizedBox(height: 8),
      ProtocolSelector(
        value: protocol,
        onChanged: editable
            ? (v) async {
                setState(() {
                  protocol = v;
                  clearProbe();
                });
                await save();
              }
            : null,
      ),
      field('host', '地址'),
      field('proxyPort', '端口', numeric: true),
      field('username', '用户名（选填）'),
      if (protocol != 'SOCKS4') field('password', '密码（选填）'),
    ]),
  ];
  List<Widget> cachePage() => [
    stoppedHint,
    card([
      heading('磁盘缓存'),
      const SizedBox(height: 20),
      Text(size('diskBytes'), style: Theme.of(context).textTheme.headlineLarge),
      const Divider(height: 32),
      row('资源数量', Text(stats['diskEntries'] ?? '0')),
      OutlinedButton(
        onPressed: editable
            ? () => perform(() async {
                if (await confirm('清理缓存', '删除已下载资源并归零计数，保留证书和设置。')) {
                  await call('clear');
                }
              })
            : null,
        child: const Text('清理缓存'),
      ),
    ]),
    card([
      heading('内存缓存'),
      const SizedBox(height: 20),
      Text(
        size('memoryBytes'),
        style: Theme.of(context).textTheme.headlineLarge,
      ),
      const SizedBox(height: 20),
      LinearProgressIndicator(
        value: memoryFraction,
        backgroundColor: Theme.of(context).dividerColor,
        borderRadius: BorderRadius.circular(6),
      ),
      field('memoryMiB', '容量上限', numeric: true),
      const SizedBox(height: 10),
      const Text('设为 0 时关闭内存缓存'),
      const SizedBox(height: 10),
      Text('内存命中 ${stats['memoryHits'] ?? '0'}'),
    ]),
  ];
  List<Widget> certificatePage() => [
    card([
      heading('当前证书'),
      const SizedBox(height: 24),
      const Text('GBF Flash Cache CA'),
      const SizedBox(height: 8),
      Text(switch (ca['state']) {
        'valid' => '已生成',
        'invalid' => '无效或已过期',
        'missing' => '尚未生成',
        _ => '正在读取…',
      }),
      if (ca['notAfter'] != null) Text('有效期至 ${ca['notAfter']}'),
      const Divider(height: 32),
      const Text('安装后，仍需浏览器信任此证书。'),
      const SizedBox(height: 20),
      FilledButton(
        onPressed: busy
            ? null
            : () => perform(() async {
                await call('ca');
                ca = await call('ca_status');
                toast('CA 证书已导出');
              }),
        child: const Text('导出 CA 证书'),
      ),
      const SizedBox(height: 12),
      OutlinedButton(
        onPressed: busy
            ? null
            : () => perform(() async {
                await call('certificateSettings');
              }),
        child: const Text('打开系统证书设置'),
      ),
    ]),
    card([
      heading('重新生成'),
      const SizedBox(height: 16),
      const Text('重新生成后，需要重新安装并信任新证书。旧证书可在系统设置中移除。'),
      const SizedBox(height: 20),
      OutlinedButton(
        onPressed: editable
            ? () => perform(() async {
                if (!await confirm(
                  '重新生成 CA 证书',
                  '使用此应用的浏览器和其它设备都需要重新安装新证书。是否继续？',
                )) {
                  return;
                }
                ca = await call('ca_regenerate', {
                  'fingerprint': ca['fingerprint'] ?? '',
                });
                toast('新证书已生成，请导出并重新安装');
              })
            : null,
        style: OutlinedButton.styleFrom(
          foregroundColor: Theme.of(context).colorScheme.error,
        ),
        child: const Text('重新生成证书'),
      ),
      if (running) const Text('请先停止服务。'),
    ]),
    const Text('请勿分享包含证书私钥的应用数据。'),
  ];
  List<Widget> settingsPage() => [
    card([
      heading('外观'),
      const SizedBox(height: 12),
      row(
        '主题',
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
          onSelected: (value) {
            if (value == themeMode) return;
            unawaited(
              perform(() async {
                final args = {'themeMode': value.name};
                await call('settings', args);
                if (!mounted) return;
                saved = {...saved, ...args};
                setState(() => themeMode = value);
                widget.onThemeChanged?.call(value);
              }, refreshStatus: false),
            );
          },
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 12),
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
      ),
    ]),
    card([
      heading('日志'),
      const SizedBox(height: 12),
      SwitchListTile(
        title: const Text('保留日志'),
        value: keepLogs,
        subtitle: const Text('未开启时，下次启动会清理旧日志。'),
        onChanged: !ready || busy
            ? null
            : (v) => perform(() async {
                final args = {'keepLogs': '$v'};
                await call('settings', args);
                saved = {...saved, ...args};
                if (mounted) setState(() => keepLogs = v);
              }, refreshStatus: false),
      ),
      const Divider(),
      row(
        '导出日志',
        const Icon(Icons.chevron_right),
        onTap: editable
            ? () => perform(() async {
                await call('export');
                toast('日志已导出');
              })
            : null,
      ),
      if (running) const Text('停止服务后可导出日志。'),
    ]),
    card([
      heading('帮助'),
      const SizedBox(height: 12),
      entry('使用说明'),
      const Divider(),
      row(
        '关于',
        const Icon(Icons.chevron_right),
        onTap: () => showDialog<void>(
          context: context,
          builder: (context) => AlertDialog(
            title: const Text('GBF Flash Cache'),
            content: SingleChildScrollView(
              child: Column(
                mainAxisSize: MainAxisSize.min,
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Image.asset('assets/app-icon.png', width: 48, height: 48),
                  const SizedBox(height: 12),
                  Text(saved['version'] ?? ''),
                  const Text('Copyright © 2026 ErinnerMO · MIT'),
                  const SizedBox(height: 16),
                  const Text('非官方工具，与游戏运营方无隶属关系。软件按现状提供，不保证提速或持续可用。'),
                ],
              ),
            ),
            actions: [
              TextButton(
                onPressed: () async {
                  Map<String, String> result;
                  try {
                    result = await call('license');
                  } catch (e) {
                    if (mounted) setState(() => error = message(e));
                    return;
                  }
                  final text = result['text'] ?? '';
                  if (!context.mounted) return;
                  showDialog<void>(
                    context: context,
                    builder: (context) => AlertDialog(
                      title: const Text('MIT 许可证'),
                      content: SingleChildScrollView(
                        child: SelectableText(text),
                      ),
                      actions: [
                        TextButton(
                          onPressed: () => Navigator.pop(context),
                          child: const Text('关闭'),
                        ),
                      ],
                    ),
                  );
                },
                child: const Text('MIT 许可证'),
              ),
              TextButton(
                onPressed: () => Navigator.pop(context),
                child: const Text('关闭'),
              ),
            ],
          ),
        ),
      ),
    ]),
  ];
  @override
  Widget build(BuildContext context) => PopScope(
    canPop: page.isEmpty,
    onPopInvokedWithResult: (didPop, _) {
      if (!didPop) back();
    },
    child: Scaffold(
      appBar: AppBar(
        automaticallyImplyLeading: false,
        leading: page.isEmpty
            ? null
            : IconButton(
                onPressed: back,
                icon: const Icon(Icons.arrow_back),
                tooltip: '返回',
              ),
        title: page.isEmpty
            ? Row(
                children: [
                  Image.asset('assets/app-icon.png', width: 36, height: 36),
                  const SizedBox(width: 10),
                  const Flexible(child: Text('GBF Flash Cache')),
                ],
              )
            : Text(page),
        actions: page.isEmpty
            ? [
                IconButton(
                  onPressed: !ready ? null : () => open('设置'),
                  icon: const Icon(Icons.settings_outlined),
                  tooltip: '设置',
                ),
              ]
            : null,
      ),
      body: SafeArea(
        child: ListView(
          key: PageStorageKey(page),
          padding: const EdgeInsets.all(16),
          children: [
            if (visibleError.isNotEmpty)
              Padding(
                padding: const EdgeInsets.only(bottom: 16),
                child: Text(
                  visibleError,
                  style: TextStyle(color: Theme.of(context).colorScheme.error),
                ),
              ),
            ...switch (page) {
              '上游代理设置' => connectionPage(),
              '监听端口' => portPage(),
              '缓存设置' => cachePage(),
              'CA 证书' => certificatePage(),
              '设置' => settingsPage(),
              '使用说明' => [const HelpGuide(mobile: true)],
              _ => home(),
            },
          ],
        ),
      ),
    ),
  );
  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    timer?.cancel();
    for (final c in fields.values) {
      c.dispose();
    }
    super.dispose();
  }
}
