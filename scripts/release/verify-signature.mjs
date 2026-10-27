import {createPrivateKey,createPublicKey,createHash} from 'node:crypto';
import {readFileSync,statSync,createReadStream} from 'node:fs';
import {join} from 'node:path';
import {verify_manifest} from '../../bridge/bootstrap.mjs';
const directory=process.argv[2];
if(!directory || !process.env.SAVIOR_RELEASE_SIGNING_KEY)throw Error('production_signing_identity_required');
const key=createPublicKey(createPrivateKey(process.env.SAVIOR_RELEASE_SIGNING_KEY)).export({type:'spki',format:'pem'});
if(key!==readFileSync(join(directory,'release-public.pem'),'utf8'))throw Error('release_signer_mismatch');
const bytes=readFileSync(join(directory,'runtime-manifest.json'));const signature=readFileSync(join(directory,'runtime-manifest.sig'));
const version=JSON.parse(readFileSync('package.json')).version;
for(const target of ['darwin-arm64','linux-x64','win32-x64']){
 const manifest=verify_manifest(bytes,signature,key,version,target);
 for(const file of manifest.targets[target]){
  const path=join(directory,file.asset);if(statSync(path).size!==file.bytes)throw Error('artifact_size_mismatch');
  const hash=createHash('sha256');for await(const chunk of createReadStream(path))hash.update(chunk);
  if(hash.digest('hex')!==file.sha256)throw Error('artifact_digest_mismatch');
 }
}
process.stdout.write('production manifest signature and artifacts verified\n');
