#!/usr/bin/env python3
"""Build the application native library for Android; SDK/NDK must already be installed."""
import os
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parents[1]
target_dir = (root / Path(os.environ.get('CARGO_TARGET_DIR') or root / 'target')).resolve()
sdk = Path(os.environ.get('ANDROID_SDK_ROOT', '/opt/coder-cache/android-sdk'))
ndk = Path(os.environ.get('ANDROID_NDK_HOME', sdk / 'ndk/28.2.13676358'))
tools = ndk / 'toolchains/llvm/prebuilt/linux-x86_64/bin'
for target, compiler in [('aarch64-linux-android', 'aarch64-linux-android'),
                         ('x86_64-linux-android', 'x86_64-linux-android')]:
    env = os.environ.copy()
    env['CARGO_TARGET_DIR'] = str(target_dir)
    env[f'CARGO_TARGET_{target.upper().replace("-", "_")}_LINKER'] = str(tools / f'{compiler}26-clang')
    env[f'CC_{target.replace("-", "_")}'] = str(tools / f'{compiler}26-clang')
    env[f'AR_{target.replace("-", "_")}'] = str(tools / 'llvm-ar')
    flags = env.get('CARGO_ENCODED_RUSTFLAGS', '').split('\x1f') if env.get('CARGO_ENCODED_RUSTFLAGS') else []
    flags += ['-C', 'link-arg=-Wl,-z,max-page-size=16384',
              f'--remap-path-prefix={Path.home()}=/build-home',
              f'--remap-path-prefix={root}=/gfc']
    env['CARGO_ENCODED_RUSTFLAGS'] = '\x1f'.join(flags)
    subprocess.run(['cargo', 'build', '-p', 'gbf-flash-cache-app', '--locked', '--release', '--target', target],
                   cwd=root, env=env, check=True)
    print(target_dir / target / 'release/libgbf_flash_cache_app.so', flush=True)
