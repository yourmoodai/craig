"""Run the installed prompt-box harness; unavailable host coverage blocks publication."""
import os
from pathlib import Path
import platform
import subprocess
import sys

import zipfile
import tempfile
from runtime_cache import seed

env=dict(os.environ)
env['CRAIG_MODEL_DIR']=str(Path('.cache/models/mudes-en-base').resolve())
if not Path(env['CRAIG_MODEL_DIR']).exists():
    subprocess.run(['node','scripts/fetch-model.mjs'],check=True,timeout=600)
with tempfile.TemporaryDirectory(prefix='craig installed ') as directory:
    root=Path(directory)
    with zipfile.ZipFile(next(Path('.cache/release').glob('craig-*.zip'))) as bundle:
        bundle.extractall(root)
        for entry in bundle.infolist():
            if entry.external_attr >> 16: (root/entry.filename).chmod(entry.external_attr >> 16)
    env['CRAIG_PLUGIN_ROOT']=str(root)
    cache=root/'cache';cache.mkdir()
    env['XDG_CACHE_HOME']=str(cache);env['LOCALAPPDATA']=str(cache)
    target={'Darwin':'darwin-arm64','Linux':'linux-x64','Windows':'win32-x64'}[platform.system()]
    seed(root,Path('.cache/release'),cache,target)
    subprocess.run([sys.executable,'scripts/test-installed-mod.py'],env=env,check=True,timeout=180)
