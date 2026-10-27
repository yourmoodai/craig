"""Publish a qualified main-branch tag without overwriting any released artifact."""
import json
import os
from pathlib import Path
import shutil
import subprocess
version=json.loads(Path('package.json').read_text())['version']
tag='v'+version
if os.environ.get('GITHUB_REF')!='refs/tags/'+tag:raise SystemExit('release tag/version mismatch')
subprocess.run(['git','merge-base','--is-ancestor','HEAD','origin/main'],check=True)
if not os.environ.get('SAVIOR_RELEASE_SIGNING_KEY'):raise SystemExit('release signing key required')
subprocess.run(['node','scripts/release/verify-signature.mjs','.cache/release'],check=True)
assets=[p for p in Path('.cache/release').glob('*') if p.is_file()]
for artifact in json.loads(Path('models/catalog.json').read_text())['models'][0]['artifacts']:
    source=Path('.cache/models/mudes-en-base')/artifact['file']
    destination=Path('.cache/release')/('model-'+artifact['file'])
    shutil.copy2(source,destination);assets.append(destination)
subprocess.run(['gh','release','create',tag,'--verify-tag','--title','Savior '+version,'--generate-notes',*[str(p) for p in assets]],check=True)
print('release published; update marketplace only after downloaded assets pass verification')
