#!/usr/bin/env python3
"""Regenerate notices in isolation; no Cargo invocation or release build."""
import json
import copy
from pathlib import Path
import runpy
import tempfile
from unittest.mock import patch

with tempfile.TemporaryDirectory() as temporary:
    root = Path(temporary)
    source = root / 'serde'
    source.mkdir()
    (source / 'LICENSE-APACHE').write_text('Apache test text')
    out = root / 'licenses'
    (out / 'removed-1.0').mkdir(parents=True)
    (out / 'removed-1.0/LICENSE').write_text('obsolete')
    (out / 'serde-1.0').mkdir()
    (out / 'serde-1.0/OLD-NOTICE').write_text('obsolete')
    metadata = {
        'workspace_members': ['app'],
        'resolve': {'root': 'app', 'nodes': [
            {'id': 'app', 'deps': [{'pkg': 'serde', 'dep_kinds': [{'kind': None}]}]},
            {'id': 'serde', 'deps': []},
        ]},
        'packages': [{'id': 'serde', 'name': 'serde', 'version': '1.0', 'source': 'registry',
                      'manifest_path': str(source / 'Cargo.toml'), 'repository': 'test', 'license': 'Apache-2.0'}],
    }
    script = Path(__file__).with_name('collect_licenses.py')
    for name in ['alloc-no-stdlib', 'alloc-stdlib']:
        allocator = root / name
        allocator.mkdir()
        if name == 'alloc-no-stdlib':
            (allocator / 'LICENSE').write_text('BSD allocator license')
        metadata['resolve']['nodes'][0]['deps'].append({'pkg': name, 'dep_kinds': [{'kind': None}]})
        metadata['resolve']['nodes'].append({'id': name, 'deps': []})
        metadata['packages'].append({'id': name, 'name': name, 'version': '1.0', 'source': 'registry',
                                    'manifest_path': str(allocator / 'Cargo.toml'), 'repository': 'allocator', 'license': 'BSD-3-Clause'})
    second = copy.deepcopy(metadata)
    second['resolve']['nodes'][0]['deps'].append({'pkg': 'extra', 'dep_kinds': [{'kind': None}]})
    second['resolve']['nodes'].append({'id': 'extra', 'deps': []})
    second['packages'].append({**metadata['packages'][0], 'id': 'extra', 'name': 'extra', 'source': None})
    def output(args, **kwargs):
        return json.dumps(second if args[-1] == 'x86_64-linux-android' else metadata).encode()
    with patch('sys.argv', [str(script), str(out), 'aarch64-linux-android', 'x86_64-linux-android']), patch('subprocess.check_output', side_effect=output):
        runpy.run_path(str(script), run_name='__main__')
    assert (out / 'extra-1.0/SOURCE.txt').is_file()
    assert not (out / 'removed-1.0').exists()
    assert not (out / 'serde-1.0/OLD-NOTICE').exists()
    assert (out / 'serde-1.0/LICENSE-APACHE').read_text() == 'Apache test text'
    assert (out / 'serde-1.0/SOURCE.txt').is_file()
    assert (out / 'alloc-stdlib-1.0/LICENSE').read_text() == 'BSD allocator license'
print('License regeneration excludes removed dependencies and old files')
