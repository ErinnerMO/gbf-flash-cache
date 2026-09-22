import 'dart:convert';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

class MobileProxySettings extends StatefulWidget {
  const MobileProxySettings({super.key, required this.active});
  final bool active;
  @override
  State<MobileProxySettings> createState() => _MobileProxySettingsState();
}

class _MobileProxySettingsState extends State<MobileProxySettings> {
  static const channel = MethodChannel('gbf/core');
  String mode = 'all', error = '';
  Set<String> included = {}, excluded = {};
  Future<List<Map<String, dynamic>>>? apps;
  bool loading = true, saving = false, loaded = false;
  bool get editable => loaded && !loading && !saving && !widget.active;
  Future<Map<String, String>> call(
    String op, [
    Map<String, String>? args,
  ]) async => Map<String, String>.from(
    await channel.invokeMapMethod<String, String>(op, args) ?? {},
  );
  String message(Object e) =>
      e is PlatformException ? (e.message ?? e.code) : e.toString();
  @override
  void initState() {
    super.initState();
    load();
  }

  Future<void> load() async {
    try {
      final values = await call('proxy_settings');
      if (!mounted) return;
      setState(() {
        loaded = true;
        mode = values['mode'] ?? 'all';
        included = Set<String>.from(
          jsonDecode(values['included'] ?? '[]') as List,
        );
        excluded = Set<String>.from(
          jsonDecode(values['excluded'] ?? '[]') as List,
        );
      });
    } catch (e) {
      if (mounted) setState(() => error = message(e));
    } finally {
      if (mounted) setState(() => loading = false);
    }
  }

  Future<List<Map<String, dynamic>>> loadApps() async {
    try {
      final available = await call('proxy_apps');
      return (jsonDecode(available['apps'] ?? '[]') as List)
          .map((e) => Map<String, dynamic>.from(e as Map))
          .toList();
    } catch (_) {
      apps = null; // A failed scan can be retried by reopening the picker.
      rethrow;
    }
  }

  Future<void> choose(String title, Set<String> selected) async {
    if (!editable) return;
    await Navigator.of(context).push<Set<String>>(
      MaterialPageRoute(
        builder: (_) => _ApplicationPicker(
          title: title,
          apps: apps ??= loadApps(),
          selected: selected,
          save: (values) => persist(
            mode,
            title == '包含应用' ? values : included,
            title == '排除应用' ? values : excluded,
          ),
        ),
      ),
    );
    if (!mounted) return;
    setState(() {});
  }

  Future<void> persist(
    String nextMode,
    Set<String> nextIncluded,
    Set<String> nextExcluded,
  ) async {
    await call('proxy_save', {
      'mode': nextMode,
      'included': jsonEncode(nextIncluded.toList()),
      'excluded': jsonEncode(nextExcluded.toList()),
    });
    // Update only after persistence succeeds, even if the page was left meanwhile.
    mode = nextMode;
    included = {...nextIncluded};
    excluded = {...nextExcluded};
  }

  Future<void> changeMode(String value) async {
    if (!editable) return;
    setState(() {
      saving = true;
      error = '';
    });
    try {
      await persist(value, included, excluded);
    } catch (e) {
      if (mounted) error = message(e);
    } finally {
      if (mounted) setState(() => saving = false);
    }
  }

  @override
  Widget build(BuildContext context) => PopScope(
    canPop: !saving,
    child: Scaffold(
      appBar: AppBar(
        title: const Text('系统代理设置'),
        leading: IconButton(
          tooltip: '返回',
          icon: const Icon(Icons.arrow_back),
          onPressed: saving ? null : () => Navigator.pop(context),
        ),
      ),
      body: ListView(
        padding: const EdgeInsets.all(16),
        children: [
          if (loading || saving) const LinearProgressIndicator(),
          if (widget.active)
            const Padding(
              padding: EdgeInsets.only(bottom: 16),
              child: Text('停止服务后可修改设置。'),
            ),
          if (error.isNotEmpty)
            Padding(
              padding: const EdgeInsets.only(bottom: 16),
              child: Text(
                error,
                style: TextStyle(color: Theme.of(context).colorScheme.error),
              ),
            ),
          Card(
            child: Padding(
              padding: const EdgeInsets.all(20),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.stretch,
                children: [
                  Text(
                    '代理应用范围',
                    style: Theme.of(context).textTheme.titleMedium,
                  ),
                  const SizedBox(height: 20),
                  DropdownButtonFormField<String>(
                    borderRadius: BorderRadius.circular(12),
                    key: ValueKey('$mode:$saving'),
                    initialValue: mode,
                    isExpanded: true,
                    items: const [
                      DropdownMenuItem(value: 'all', child: Text('全部应用')),
                      DropdownMenuItem(value: 'include', child: Text('包含应用')),
                      DropdownMenuItem(value: 'exclude', child: Text('排除应用')),
                    ],
                    onChanged: editable ? (value) => changeMode(value!) : null,
                  ),
                  const SizedBox(height: 16),
                  Text(switch (mode) {
                    'include' => '仅包含名单生效。',
                    'exclude' => '仅排除名单生效。',
                    _ => '两份名单均不生效。',
                  }),
                  const Divider(height: 32),
                  ListTile(
                    contentPadding: EdgeInsets.zero,
                    title: const Text('包含应用'),
                    subtitle: Text('${included.length} 个应用'),
                    trailing: const Icon(Icons.chevron_right),
                    onTap: editable ? () => choose('包含应用', included) : null,
                  ),
                  ListTile(
                    contentPadding: EdgeInsets.zero,
                    title: const Text('排除应用'),
                    subtitle: Text('${excluded.length} 个应用'),
                    trailing: const Icon(Icons.chevron_right),
                    onTap: editable ? () => choose('排除应用', excluded) : null,
                  ),
                  const Text('切换范围会保留两份名单。本应用始终自动绕过。'),
                ],
              ),
            ),
          ),
        ],
      ),
    ),
  );
}

class _ApplicationPicker extends StatefulWidget {
  const _ApplicationPicker({
    required this.title,
    required this.apps,
    required this.selected,
    required this.save,
  });
  final String title;
  final Future<List<Map<String, dynamic>>> apps;
  final Future<void> Function(Set<String>) save;
  final Set<String> selected;
  @override
  State<_ApplicationPicker> createState() => _ApplicationPickerState();
}

class _ApplicationPickerState extends State<_ApplicationPicker> {
  late final Set<String> selected = {...widget.selected};
  String query = '', error = '';
  bool saving = false;
  List<Map<String, dynamic>>? apps;
  @override
  void initState() {
    super.initState();
    widget.apps.then(
      (value) {
        if (mounted) setState(() => apps = value);
      },
      onError: (Object e) {
        if (mounted) {
          setState(
            () => error = e is PlatformException
                ? (e.message ?? e.code)
                : e.toString(),
          );
        }
      },
    );
  }

  Future<void> save(String package, bool value) async {
    final next = {...selected};
    if (value) {
      next.add(package);
    } else {
      next.remove(package);
    }
    setState(() {
      saving = true;
      error = '';
    });
    try {
      await widget.save(next);
      if (mounted) {
        setState(() {
          selected.clear();
          selected.addAll(next);
        });
      }
    } catch (e) {
      if (mounted) {
        setState(
          () => error = e is PlatformException
              ? (e.message ?? e.code)
              : e.toString(),
        );
      }
    } finally {
      if (mounted) setState(() => saving = false);
    }
  }

  @override
  Widget build(BuildContext context) {
    final entries =
        [
              ...?(apps),
              for (final package in selected)
                if (!(apps ?? []).any((app) => app['package'] == package))
                  {'package': package, 'label': '未安装的应用'},
            ]
            .where(
              (app) => '${app['label']} ${app['package']}'
                  .toLowerCase()
                  .contains(query.toLowerCase()),
            )
            .toList();
    return PopScope(
      canPop: !saving,
      child: Scaffold(
        appBar: AppBar(
          title: Text(widget.title),
          leading: IconButton(
            tooltip: '返回',
            icon: const Icon(Icons.arrow_back),
            onPressed: saving ? null : () => Navigator.pop(context),
          ),
        ),
        body: Column(
          children: [
            if ((apps == null && error.isEmpty) || saving)
              const LinearProgressIndicator(),
            if (error.isNotEmpty)
              Padding(
                padding: const EdgeInsets.all(16),
                child: Text(
                  error,
                  style: TextStyle(color: Theme.of(context).colorScheme.error),
                ),
              ),
            Padding(
              padding: const EdgeInsets.all(16),
              child: TextField(
                decoration: const InputDecoration(
                  labelText: '搜索应用',
                  prefixIcon: Icon(Icons.search),
                ),
                enabled: !saving,
                onChanged: (value) => setState(() => query = value),
              ),
            ),
            Expanded(
              child: ListView.builder(
                itemCount: entries.length,
                itemBuilder: (_, index) {
                  final app = entries[index];
                  final package = app['package'] as String;
                  return CheckboxListTile(
                    title: Text(app['label'] as String),
                    subtitle: Text(package),
                    value: selected.contains(package),
                    onChanged: saving ? null : (value) => save(package, value!),
                  );
                },
              ),
            ),
          ],
        ),
      ),
    );
  }
}
