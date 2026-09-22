import 'dart:io';

void checkVersion(Directory root, String expected) {
  for (final path in ['core/Cargo.toml', 'app/native/Cargo.toml']) {
    final text = File('${root.path}/$path').readAsStringSync();
    final package = RegExp(
      r'^\[package\]\s*\n([\s\S]*?)(?=^\[|$(?![\s\S]))',
      multiLine: true,
    ).firstMatch(text)?.group(1);
    final version = package == null
        ? null
        : RegExp(
            r'^version\s*=\s*"([^"]+)"\s*$',
            multiLine: true,
          ).firstMatch(package)?.group(1);
    if (version != expected) {
      throw StateError('$path version $version != requested $expected');
    }
  }
  final pubspec = File('${root.path}/app/pubspec.yaml').readAsStringSync();
  final version = RegExp(
    r'^version:\s*([^+\s]+)',
    multiLine: true,
  ).firstMatch(pubspec)?.group(1);
  if (version != expected) {
    throw StateError(
      'app/pubspec.yaml version $version != requested $expected',
    );
  }
}

void main(List<String> args) {
  if (args.length != 1) throw ArgumentError('Expected release version');
  checkVersion(File.fromUri(Platform.script).parent.parent.parent, args.single);
  stdout.writeln('Release version verified: ${args.single}');
}
