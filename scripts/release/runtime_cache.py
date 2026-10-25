"""Seed candidate assets without executing them; bootstrap still verifies signatures."""
import hashlib
import json
from pathlib import Path
import shutil

def seed(plugin, assets, cache, target):
    raw=(plugin/'runtime-manifest.json').read_bytes()
    manifest=json.loads(raw)
    directory=cache/'craig/releases'/manifest['version']/target/hashlib.sha256(raw).hexdigest()
    directory.mkdir(parents=True,exist_ok=True)
    for entry in manifest['targets'][target]:
        source=assets/entry['asset']
        assert source.stat().st_size==entry['bytes']
        assert hashlib.sha256(source.read_bytes()).hexdigest()==entry['sha256']
        shutil.copy2(source,directory/entry['name'])
    return directory
