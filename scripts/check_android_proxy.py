#!/usr/bin/env python3
"""Run Android scope checks without a device or emulator."""
from pathlib import Path
import subprocess
import tempfile
root = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix='gbf-proxy-check-') as classes:
    subprocess.run(['javac', '-d', classes,
        str(root / 'app/android/app/src/main/java/dev/gbfcache/flashcache/ProxyScope.java'),
        str(root / 'app/android/checks/ProxyScopeCheck.java')], check=True)
    subprocess.run(['java', '-ea', '-cp', classes, 'dev.gbfcache.flashcache.ProxyScopeCheck'], check=True)
