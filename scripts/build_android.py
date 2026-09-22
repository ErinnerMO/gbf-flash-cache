#!/usr/bin/env python3
"""Build signed ARM64/x86_64 Android APKs with the shared Rust core."""
from pathlib import Path
import os
import secrets
import shutil
import subprocess

repo = Path(__file__).resolve().parents[1]
root = repo / "app"
env = os.environ.copy()
target_dir = Path(env.get('CARGO_TARGET_DIR') or repo / 'target')
target_dir = (repo / target_dir).resolve()
env['CARGO_TARGET_DIR'] = str(target_dir)
env['ANDROID_HOME'] = env.get('GBF_ANDROID_SDK', '/opt/coder-cache/android-sdk')
env['ANDROID_SDK_ROOT'] = env['ANDROID_HOME']
env.setdefault('ANDROID_NDK_HOME', str(Path(env['ANDROID_HOME']) / 'ndk/28.2.13676358'))
def run(*args):
    subprocess.run(list(map(str, args)), cwd=root, env=env, check=True)
run('python3', repo / 'scripts/build_native_android.py')
for abi, target in [('arm64-v8a', 'aarch64-linux-android'), ('x86_64', 'x86_64-linux-android')]:
    destination = root / 'android/app/src/main/jniLibs' / abi
    # Remove stale packaged native libraries from earlier builds.
    if destination.exists(): shutil.rmtree(destination)
    destination.mkdir(parents=True, exist_ok=True)
    shutil.copy2(target_dir / target / 'release/libgbf_flash_cache_app.so', destination)
assets = root / 'android/app/src/main/assets'
assets.mkdir(exist_ok=True)
shutil.copyfile(repo / 'LICENSE', assets / 'gfc-license.txt')
licenses = root / 'build/native/licenses'
run('python3', repo / 'scripts/collect_licenses.py', licenses, 'aarch64-linux-android', 'x86_64-linux-android')
texts = [repo / 'LICENSE']
texts += sorted(p for p in licenses.rglob('*') if p.is_file())
(assets / 'native-notices.txt').write_text('\n\n'.join(
    f'{p.name}\n{p.read_text(errors="replace")}' for p in texts), encoding='utf-8')
# A persistent local release key; never included in source, APKs or build output.
keydir = Path(env.get('GBF_ANDROID_KEY_DIR', Path.home() / '.local/share/gbf-flash-cache/android-signing'))
keydir.mkdir(parents=True, exist_ok=True, mode=0o700)
password_file = keydir / 'password'
keystore = keydir / 'release.jks'
if not password_file.exists():
    if keystore.exists():
        raise RuntimeError('Signing password missing; preserve the existing keystore')
    fd = os.open(password_file, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    with os.fdopen(fd, 'w') as f:
        f.write(secrets.token_urlsafe(36))
env['GBF_ANDROID_KEYSTORE_PASSWORD'] = password_file.read_text()
env['GBF_ANDROID_KEYSTORE'] = str(keystore)
if not keystore.exists():
    run('keytool', '-genkeypair', '-keystore', keystore,
        '-storepass:env', 'GBF_ANDROID_KEYSTORE_PASSWORD', '-keypass:env', 'GBF_ANDROID_KEYSTORE_PASSWORD',
        '-alias', 'gbf-flash-cache', '-keyalg', 'RSA', '-keysize', '3072', '-validity', '10000',
        '-dname', 'CN=ErinnerMO')
    keystore.chmod(0o600)
run(env.get('FLUTTER', 'flutter'), 'pub', 'get')
run(Path(env.get('FLUTTER', 'flutter')).with_name('dart'), root / 'tool/prepare_release.dart')
run(env.get('FLUTTER', 'flutter'), 'build', 'apk', '--release', '--no-pub',
    '--split-debug-info=' + str(root / 'build/symbols'),
    '--split-per-abi', '--target-platform', 'android-arm64,android-x64', '--build-name=1.14.98', '--build-number=11498')
