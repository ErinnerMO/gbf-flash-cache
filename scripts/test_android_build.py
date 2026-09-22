#!/usr/bin/env python3
"""Check native output selection without compiling, signing or building an APK."""
import os
from pathlib import Path
import runpy
import shutil
import tempfile
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parent
TARGETS = [('arm64-v8a', 'aarch64-linux-android'), ('x86_64', 'x86_64-linux-android')]
LIBRARY = 'libgbf_flash_cache_app.so'


class Packaged(Exception):
    pass


def check(setting):
    with tempfile.TemporaryDirectory() as directory:
        repo = Path(directory) / 'repo'
        (repo / 'scripts').mkdir(parents=True)
        (repo / 'app').mkdir()
        (repo / 'LICENSE').write_text('MIT test license')
        for name in ['build_android.py', 'build_native_android.py']:
            shutil.copy2(SOURCE / name, repo / 'scripts' / name)
        selected = {'default': repo / 'target', 'absolute': Path(directory) / 'external',
                    'relative': repo / 'relative-output'}[setting]
        for _, target in TARGETS:
            old = repo / 'target' / target / 'release' / LIBRARY
            old.parent.mkdir(parents=True)
            old.write_bytes(b'old')
        environment = dict(os.environ)
        environment.pop('CARGO_TARGET_DIR', None)
        if setting != 'default':
            environment['CARGO_TARGET_DIR'] = str(selected) if setting == 'absolute' else 'relative-output'
        builds = []

        def run(args, *, cwd, env, check):
            if args[0] == 'cargo':
                assert Path(cwd) == repo
                assert Path(env['CARGO_TARGET_DIR']) == selected
                assert f'--remap-path-prefix={repo}=/gfc' in env['CARGO_ENCODED_RUSTFLAGS']
                target = args[args.index('--target') + 1]
                output = selected / target / 'release' / LIBRARY
                output.parent.mkdir(parents=True, exist_ok=True)
                output.write_bytes(b'fresh')
                builds.append(target)
            elif str(args[1]).endswith('build_native_android.py'):
                with patch.dict(os.environ, env, clear=True):
                    runpy.run_path(str(args[1]), run_name='__main__')
            elif str(args[1]).endswith('collect_licenses.py'):
                assert args[-2:] == ['aarch64-linux-android', 'x86_64-linux-android']
                assert (repo / 'app/android/app/src/main/assets/gfc-license.txt').read_text() == 'MIT test license'
                raise Packaged()  # Stop before notices, signing and Flutter build.
            else:
                raise AssertionError(args)

        with patch.dict(os.environ, environment, clear=True), patch('subprocess.run', run):
            try:
                runpy.run_path(str(repo / 'scripts/build_android.py'), run_name='__main__')
            except Packaged:
                pass
            else:
                raise AssertionError('packaging checkpoint not reached')
        assert builds == [target for _, target in TARGETS]
        for abi, _ in TARGETS:
            assert (repo / 'app/android/app/src/main/jniLibs' / abi / LIBRARY).read_bytes() == b'fresh'


if __name__ == '__main__':
    for setting in ['default', 'absolute', 'relative']:
        check(setting)
    print('Android native output selection: 3 cases, both ABIs passed')
