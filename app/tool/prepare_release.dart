// Run after pub get, then build with --no-pub. Only generated metadata changes.
import 'dart:convert';
import 'dart:io';

void main() {
  final app = File.fromUri(Platform.script).parent.parent;
  final config = File('${app.path}/.dart_tool/package_config.json');
  final data = jsonDecode(config.readAsStringSync()) as Map<String, dynamic>;
  final packages = data['packages'] as List<dynamic>;
  final language = packages.firstWhere(
    (p) => p['name'] == 'gbf_flash_cache',
  )['languageVersion'];
  packages.removeWhere((p) => p['name'] == 'gfc_build');
  // Flutter resolves the generated registrant to this stable package URI,
  // preserving registration without embedding the build machine's absolute path.
  packages.add({
    'name': 'gfc_build',
    'rootUri': 'flutter_build/',
    'packageUri': './',
    'languageVersion': language,
  });
  config.writeAsStringSync(
    '${const JsonEncoder.withIndent('  ').convert(data)}\n',
  );
}
