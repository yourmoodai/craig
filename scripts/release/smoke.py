"""Exercise the packaged runtime without developer PATH tools or model overrides."""
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import tempfile
import urllib.request
import zipfile
from bridge_check import request as bridge_request
from runtime_cache import seed

release=Path(sys.argv[1]).resolve()
archive=next(release.glob('savior-*.zip'))
with tempfile.TemporaryDirectory(prefix='savior release ') as temporary:
    root=Path(temporary).resolve()
    with zipfile.ZipFile(archive) as bundle:
        bundle.extractall(root)
        for entry in bundle.infolist():
            mode=entry.external_attr>>16
            if mode: (root/entry.filename).chmod(mode)
    manifest=json.loads((root/'release.json').read_text())
    for name,expected in manifest['files'].items():
        with (root/name).open('rb') as stream:
            assert hashlib.file_digest(stream,'sha256').hexdigest()==expected,name
    target={'Darwin':'darwin-arm64','Linux':'linux-x64','Windows':'win32-x64'}[platform.system()]
    suffix='.exe' if os.name=='nt' else ''
    native=root/'runtime'/target
    node=native/('node'+suffix)
    cache=root/'cache';cache.mkdir()
    env={k:v for k,v in os.environ.items() if not k.startswith('SAVIOR_')}
    env.update(PATH=str(native), XDG_CACHE_HOME=str(cache), LOCALAPPDATA=str(cache))
    if '--download' not in sys.argv:
        seed(root,release,cache,target)
    setup=subprocess.run([str(node),str(root/'bridge/bootstrap.mjs')],env=env,text=True,capture_output=True,check=True,timeout=600)
    prepared=json.loads(setup.stdout);assert prepared['status']=='ready'
    native=Path(prepared['runtime_dir']);env['SAVIOR_RUNTIME_DIR']=str(native)
    # Candidate assets are not public yet. Seed verified model bytes in the same
    # cache layout; prepare still verifies every digest, offline, with no override.
    model=manifest['model'];directory=cache/'savior/models'/model['provenance']['upstream_revision'];directory.mkdir(parents=True)
    for artifact in model['artifacts'] if '--download' not in sys.argv else []:
        path=directory/artifact['file']
        url=model['provenance']['upstream_url']+'/resolve/'+model['provenance']['upstream_revision']+'/'+artifact['file']
        urllib.request.urlretrieve(url,path)
    subprocess.run([str(native/('savior'+suffix)),'prepare'],env=env,check=True,timeout=600)
    env['SAVIOR_MODEL_DIR']=str(directory)
    result=subprocess.run([str(node),str(root/'bridge/cli.mjs'),'rewrite'],input=json.dumps({'version':1,'text':'You idiot, do not delete the test.'}),text=True,capture_output=True,env=env,check=True,timeout=60)
    assert json.loads(result.stdout)['text']=='You ya ya ya, do not delete the test.'
    profile=root/'profile';profile.mkdir();env['CLAUDE_CONFIG_DIR']=str(profile)
    def request(command):
        return bridge_request([str(node),str(root/'bridge/review.mjs')],command,env,'smoke_'+command['command'])
    started=False
    try:
        state=request({'command':'status'})
        started=True
        assert state['consent']['live'] is False and state['consent']['history'] is False
    finally:
        if started:
            failed=sys.exc_info()[0] is not None
            try: request({'command':'shutdown'})
            except Exception:
                if not failed: raise
print('packaged replacement and private worker passed without developer tools')
