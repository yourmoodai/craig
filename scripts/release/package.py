"""Assemble immutable release artifacts; no compilation occurs on user machines."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import urllib.request
import zipfile

ROOT = Path(__file__).resolve().parents[2]
NODE = '24.22.0'
TARGETS = {
    'darwin-arm64': ('tar.gz', '6a9724d9b50a5f22b4816cded441b5a8d9ddae546191a5e5b7efa5a5e2ee26d5', 'darwin-arm64'),
    'linux-x64': ('tar.gz', '28ea0a438de9a13bd7891fd758eee6c7819ee8af5442dfd8d45bd65e56d4228c', 'linux-x64-gnu'),
    'win32-x64': ('zip', 'f9683a19877e12a9b167305c8db71040d812876359de21f0d2e97c3f321f80c3', 'win32-x64-msvc'),
}
LIMIT = 210_000_000  # 210 MB, stricter than the host's 256 MiB download limit

def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()

def stage(target, destination):
    ext, expected, addon = TARGETS[target]
    suffix = '.exe' if target.startswith('win32') else ''
    directory = destination / 'runtime' / target
    directory.mkdir(parents=True, exist_ok=True)
    name = f'node-v{NODE}-{target.replace("win32", "win")}'
    archive = ROOT / '.cache' / f'{name}.{ext}'
    archive.parent.mkdir(exist_ok=True)
    if not archive.exists() or digest(archive) != expected:
        urllib.request.urlretrieve(f'https://nodejs.org/dist/v{NODE}/{archive.name}', archive)
    if digest(archive) != expected:
        raise ValueError('node checksum mismatch')
    member = name + ('/node.exe' if suffix else '/bin/node')
    with (zipfile.ZipFile(archive) if suffix else tarfile.open(archive)) as bundle:
        for filename, output in [(member, 'node'+suffix),(name+'/LICENSE','NODE-LICENSE')]:
            source = bundle.open(filename) if suffix else bundle.extractfile(filename)
            with source, (directory/output).open('wb') as stream:
                shutil.copyfileobj(source,stream)
    for binary in ['savior','savior-review']:
        shutil.copy2(ROOT/'target/release'/(binary+suffix),directory/(binary+suffix))
    for name in ['node'+suffix,'savior'+suffix,'savior-review'+suffix]:
        (directory/name).chmod(0o755)
    (destination/'native').mkdir(exist_ok=True)
    shutil.copy2(ROOT/'native'/f'savior.{addon}.node',destination/'native')
    for name in ['index.cjs','index.d.ts']:
        shutil.copy2(ROOT/'native'/name,destination/'native'/name)

def assemble(parts, destination):
    if destination.exists():
        raise ValueError('output already exists; use a fresh staging directory')
    destination.mkdir(parents=True)
    version=json.loads((ROOT/'package.json').read_text())['version']
    for target in TARGETS:
        part=parts/target
        if not (part/'runtime'/target).is_dir(): raise ValueError('missing target '+target)
        shutil.copytree(part,destination,dirs_exist_ok=True)
        suffix='.exe' if target.startswith('win32') else ''
        for name in ['node','savior','savior-review']:
            (destination/'runtime'/target/(name+suffix)).chmod(0o755)
    for name in ['hooks','bridge','src','models','.claude-plugin']:
        shutil.copytree(ROOT/name,destination/name,ignore=shutil.ignore_patterns('__pycache__'))
    for name in ['LICENSE','NOTICE','THIRD-PARTY-NOTICES.md','README.md','package.json','package-lock.json']:
        shutil.copy2(ROOT/name,destination/name)
    (destination/'docs/assets').mkdir(parents=True)
    for name in ['savior.png', 'savior.svg']:
        shutil.copy2(ROOT/'docs/assets'/name,destination/'docs/assets')
    shutil.copy2(ROOT/'docs/migration.md',destination/'docs')
    subprocess.run(['npm','ci','--omit=dev','--omit=optional','--ignore-scripts','--bin-links=false','--prefix',str(destination)],check=True)
    package=json.loads((destination/'package.json').read_text())
    for key in ['dependencies','devDependencies','scripts','napi','bin','files']:
        package.pop(key,None)
    (destination/'package.json').write_text(json.dumps(package,indent=2)+'\n')
    (destination/'package-lock.json').unlink()
    (destination/'.claude-plugin/marketplace.json').unlink()
    (destination/'hooks/distribution.ts').write_text('export const packaged: boolean = true;\n')
    files={str(p.relative_to(destination)).replace('\\','/'):digest(p) for p in sorted(destination.rglob('*')) if p.is_file()}
    manifest={'version':version,'node':NODE,'targets':list(TARGETS),'files':files,'model':json.loads((ROOT/'models/catalog.json').read_text())['models'][0]}
    runtime={'schema':1,'protocol':1,'version':version,'model':manifest['model'],'targets':{}}
    for target, (_, _, addon) in TARGETS.items():
        suffix='.exe' if target.startswith('win32') else ''
        entries=[]
        for source, name in [(destination/'runtime'/target/('savior'+suffix),'savior'+suffix),
                             (destination/'runtime'/target/('savior-review'+suffix),'savior-review'+suffix),
                             (destination/'native'/f'savior.{addon}.node','savior.node')]:
            asset=f'runtime-{version}-{target}-{name}'
            entries.append({'name':name,'asset':asset,'bytes':source.stat().st_size,'sha256':digest(source)})
            shutil.copy2(source,destination.parent/asset)
            source.unlink()
        runtime['targets'][target]=entries
    (destination/'runtime-manifest.json').write_text(json.dumps(runtime,sort_keys=True,separators=(',',':'))+'\n')
    subprocess.run(['node',str(ROOT/'scripts/release/sign.mjs'),str(destination/'runtime-manifest.json'),str(destination/'runtime-manifest.sig'),str(destination/'release-public.pem')],check=True)
    for name in ['runtime-manifest.json','runtime-manifest.sig','release-public.pem']:
        shutil.copy2(destination/name,destination.parent/name)
    manifest['files']={str(p.relative_to(destination)).replace('\\','/'):digest(p) for p in sorted(destination.rglob('*')) if p.is_file()}
    (destination/'release.json').write_text(json.dumps(manifest,indent=2)+'\n')
    output=destination.parent/f'savior-{version}.zip'
    with zipfile.ZipFile(output,'w',zipfile.ZIP_DEFLATED,compresslevel=9) as archive:
        for path in sorted(destination.rglob('*')):
            if path.is_symlink(): raise ValueError('release contains symlink')
            if path.is_file():archive.write(path,path.relative_to(destination))
    if output.stat().st_size>=LIMIT: raise ValueError('plugin archive exceeds 210 MB')
    marketplace=json.loads((ROOT/'.claude-plugin/marketplace.json').read_text())
    marketplace['plugins'][0]['source']={'source':'archive','url':f'https://github.com/yourmoodai/savior/releases/download/v{version}/{output.name}','sha256':digest(output)}
    (destination.parent/'marketplace.json').write_text(json.dumps(marketplace,indent=2)+'\n')
    (destination.parent/'SHA256SUMS').write_text(f'{digest(output)}  {output.name}\n')
    print(json.dumps({'archive':str(output),'bytes':output.stat().st_size,'sha256':digest(output)}))

if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    sub=parser.add_subparsers(dest='command',required=True)
    p=sub.add_parser('stage');p.add_argument('target',choices=TARGETS);p.add_argument('destination',type=Path)
    p=sub.add_parser('assemble');p.add_argument('parts',type=Path);p.add_argument('destination',type=Path)
    args=parser.parse_args()
    if args.command=='stage':stage(args.target,args.destination)
    else:assemble(args.parts,args.destination)
