import { spawn } from 'node:child_process';
const flags = process.platform === 'darwin' ? ['-l'] : ['-v'];
const child = spawn('/usr/bin/time',[...flags,'target/profiling/craig','rewrite'],{
  env:{...process.env,CRAIG_MODEL_DIR:process.env.CRAIG_MODEL_DIR ?? '.cache/models/mudes-en-base'},
  stdio:['pipe','ignore','inherit'],
});
child.stdin.end(JSON.stringify({version:1,text:'You idiot, fix the test without changing its assertions.'}));
child.on('error',()=>{console.error('profiling process failed');process.exitCode=1;});
child.on('exit',code=>{process.exitCode=code ?? 1;});
