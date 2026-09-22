#!/usr/bin/env python3
"""Collect source-provided license notices for the native dependency graph."""
import json
from pathlib import Path
import shutil
import subprocess
import sys

root = Path(__file__).resolve().parents[1] / "app/native"
out = Path(sys.argv[1])
def metadata_for(target):
    return json.loads(subprocess.check_output([
    'cargo', 'metadata', '--locked', '--format-version', '1',
    '--filter-platform', target,
], cwd=root))
packages = {}
for target in sys.argv[2:] or ['x86_64-pc-windows-msvc']:
    metadata = metadata_for(target)
    nodes = {n['id']: n for n in metadata['resolve']['nodes']}
    seen, pending = set(), [metadata['resolve']['root']]
    while pending:
        identity = pending.pop()
        if identity in seen:
            continue
        seen.add(identity)
        for dependency in nodes[identity]['deps']:
            if any(kind['kind'] != 'dev' for kind in dependency['dep_kinds']):
                pending.append(dependency['pkg'])
    packages.update({p['id']: p for p in metadata['packages']
                     if p['id'] in seen and p['id'] not in metadata['workspace_members']})
packages = sorted(packages.values(), key=lambda p: (p['name'], p['version']))
# These dual-licensed crates omit text from their published archives. Select Apache-2.0.
apache = next(Path(p['manifest_path']).parent / 'LICENSE-APACHE'
              for p in packages if p['name'] == 'serde')
# This output directory is dedicated to generated dependency notices.
if out.is_symlink():
    raise RuntimeError('License output must not be a symlink')
if out.exists():
    shutil.rmtree(out)
out.mkdir(parents=True)
for package in packages:
    source = Path(package['manifest_path']).parent
    destination = out / f"{package['name']}-{package['version']}"
    destination.mkdir(parents=True, exist_ok=True)
    texts = [p for p in source.rglob('*') if p.is_file() and p.name.lower().startswith(
        ('license', 'licence', 'notice', 'copying', 'copyright'))]
    if not texts:
        if package['name'] == 'alloc-stdlib' and package['license'] == 'BSD-3-Clause':
            # This subcrate omits the repo-root license; alloc-no-stdlib ships that same text.
            parent = next(p for p in packages if p['name'] == 'alloc-no-stdlib'
                          and p['repository'] == package['repository'])
            shutil.copyfile(Path(parent['manifest_path']).parent / 'LICENSE', destination / 'LICENSE')
        elif package['name'] in ('asn1-rs-impl', 'yasna', 'jni-sys-macros', 'etherparse'):
            shutil.copyfile(apache, destination / 'LICENSE-APACHE')
        else:
            raise RuntimeError(f"Missing license text: {package['name']}")
    for text in texts:
        target = destination / text.relative_to(source)
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(text, target)
    (destination / 'SOURCE.txt').write_text(
        f"{package['name']} {package['version']}\n{package['repository']}\n"
        f"License: {package['license']}\n", encoding='utf-8')
print(f'Collected notices for {len(packages)} native dependencies')
