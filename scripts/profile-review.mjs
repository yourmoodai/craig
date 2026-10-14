import {spawn} from 'node:child_process';
import {mkdtemp, writeFile, rm} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join} from 'node:path';

const judges=process.argv.slice(2);
if (judges.length!==0 && judges.length!==2) throw new Error('expected zero or two judge configuration paths');
const directory=await mkdtemp(join(tmpdir(),'craig-profile-'));
try {
  const corpus=join(directory,'corpus.json');
  await writeFile(corpus,JSON.stringify([{id:'synthetic',category:'instruction_preservation',prompts:['You idiot, do not delete the test.'],expected_replacements:['You ya ya ya, do not delete the test.'],expected:'no_concern_observed',reviewed:false}]),{mode:0o600});
  const child=spawn('/usr/bin/time',[process.platform==='darwin'?'-l':'-v','target/profiling/craig-review',...(judges.length ? ['policy-compare',corpus,...judges,join(directory,'report.json')] : ['policy-evaluate',corpus,join(directory,'report.json')])],{stdio:'inherit',env:{...process.env,CRAIG_MODEL_DIR:process.env.CRAIG_MODEL_DIR??'.cache/models/mudes-en-base'}});
  process.exitCode=await new Promise((done,reject)=>{child.on('error',reject);child.on('exit',code=>done(code??1));});
} finally {await rm(directory,{recursive:true,force:true});}
