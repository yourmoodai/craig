import {chromium,expect} from '@playwright/test';
import {spawn,spawnSync} from 'node:child_process';
import {mkdtemp,mkdir,realpath,rm,readFile,writeFile} from 'node:fs/promises';
import {createHash} from 'node:crypto';
import {tmpdir} from 'node:os';
import {join,resolve,dirname} from 'node:path';
import assert from 'node:assert/strict';
import {request} from 'node:http';

const temporary=await realpath(await mkdtemp(join(tmpdir(),'savior-inspector-e2e-')));
await mkdir(join(temporary,'profile'));
const env={...process.env,XDG_STATE_HOME:temporary,LOCALAPPDATA:temporary,CLAUDE_CONFIG_DIR:join(temporary,'profile'),SAVIOR_MODEL_DIR:resolve(process.env.SAVIOR_MODEL_DIR??'.cache/models/mudes-en-base')};
const binary=resolve('target/release/savior-review'+(process.platform==='win32'?'.exe':''));
const child=spawn(binary,['inspect'],{env,stdio:['ignore','pipe','pipe']});
let browser;
function rewrite(text){const r=spawnSync(process.execPath,['bridge/cli.mjs','rewrite'],{env,input:JSON.stringify({version:3,text}),encoding:'utf8',timeout:60000});assert.equal(r.status,0,r.stderr);const v=JSON.parse(r.stdout);assert.ok(v.trace_id,JSON.stringify(v));return v;}
try{
 const url=await new Promise((resolve,reject)=>{let buffer='';const timer=setTimeout(()=>reject(Error('inspector_start_timeout')),15000);child.once('exit',()=>{clearTimeout(timer);reject(Error('inspector_exit'));});child.stdout.on('data',b=>{buffer+=b;if(buffer.includes('\n')){clearTimeout(timer);try{resolve(JSON.parse(buffer.split('\n')[0]).url);}catch(e){reject(e);}}});});
 const location=new URL(url),token=location.hash.slice(1),origin=location.origin;
 assert.equal((await fetch(origin+'/api/status')).status,403);
 assert.equal((await fetch(origin+'/api/status',{headers:{Authorization:'Bearer '+token,Origin:'https://example.invalid'}})).status,403);
 const wrong_host=await new Promise((resolve,reject)=>{const r=request(origin+'/api/status',{headers:{Authorization:'Bearer '+token,Host:'evil.invalid'}},response=>{response.resume();resolve(response.statusCode);});r.on('error',reject);r.end();});assert.equal(wrong_host,403);
 rewrite('You idiot, keep this test.');
 browser=await chromium.launch();const page=await browser.newPage();const requests=[];
 page.on('request',r=>requests.push(r.url()));page.on('dialog',dialog=>dialog.accept());
 await page.goto(url);await expect(page).toHaveTitle("Savior · local replacements");await expect(page.locator("header .brand-icon")).toBeVisible();assert.equal(await page.locator("header .brand-icon").evaluate(image=>image.naturalWidth>0),true);await expect(page.locator('#status')).toContainText('full-text recording off');await expect(page.locator('article .original')).toHaveText('Text not recorded');
 await page.getByRole('button',{name:'Enable full traces',exact:true}).click();await expect(page.locator('#status')).toContainText('recording on');
 const result=rewrite('🌱 You idiot, keep this test.');assert.equal(result.text,'🌱 You ya ya ya, keep this test.');
 await page.getByRole('button',{name:'Refresh',exact:true}).click();await expect(page.locator('article mark').first()).toHaveText('idiot');
 const article=page.locator('article').first();await article.locator('select').nth(0).selectOption('pass');await article.locator('select').nth(1).selectOption('unresolved');await article.locator('select').nth(2).selectOption('pass');await article.locator('textarea').fill('synthetic review');await article.getByRole('button',{name:'Save human judgment'}).click();await expect(article.getByRole('button',{name:'Saved',exact:true})).toBeVisible();
 rewrite('You idiot. <img src=x onerror="window.pwned=true">');await page.getByRole('button',{name:'Refresh',exact:true}).click();await expect(page.locator('article .original').first()).toContainText('<img');assert.equal(await page.evaluate(()=>window.pwned),undefined);assert.equal(await page.locator('article img').count(),0);
 const exported=join(temporary,'export.parquet');const export_run=spawnSync(binary,['traces','export',exported],{env,encoding:'utf8'});assert.equal(export_run.status,0,export_run.stderr);assert.equal((await readFile(exported)).subarray(0,4).toString(),'PAR1');
 const downloaded=page.waitForEvent('download');await page.getByRole('button',{name:'Export Parquet',exact:true}).click();const download=await downloaded;assert.equal(download.suggestedFilename(),"savior-replacements.parquet");const download_path=await download.path();assert.equal((await readFile(download_path)).subarray(0,4).toString(),'PAR1');
 await page.getByRole('button',{name:'Delete records',exact:true}).click();await expect(page.locator('#status')).toContainText('0 events');await expect(page.locator('article')).toHaveCount(0);assert.ok((await readFile(exported)).length>0);
 assert.ok(requests.every(url=>url.startsWith(origin+'/')));
 const failed_store=spawnSync(process.execPath,['bridge/cli.mjs','rewrite'],{env:{...env,XDG_STATE_HOME:'relative',LOCALAPPDATA:'relative'},input:JSON.stringify({version:3,text:'You idiot, keep this test.'}),encoding:'utf8',timeout:60000});assert.equal(failed_store.status,0);const unrecorded=JSON.parse(failed_store.stdout);assert.equal(unrecorded.text,'You ya ya ya, keep this test.');assert.equal(unrecorded.trace_warning,'trace_not_recorded');assert.equal(unrecorded.trace_id,undefined);
 const report={schema:1,status:'passed',timestamp:new Date().toISOString(),platform:process.platform,arch:process.arch,checks:['metadata_default','explicit_consent','unicode_diff','annotation','xss_literal','authentication','origin','host','parquet_export','delete','no_external_browser_requests','trace_failure_preserves_filtered_output'],identity:{}};
 for(const file of ['crates/review/src/traces.rs','crates/review/src/inspector.rs','inspector/app.js','scripts/test-inspector.mjs'])report.identity[file]=createHash('sha256').update(await readFile(file)).digest('hex');
 const json=JSON.stringify(report,null,2)+'\n';if(process.argv[2]){const output=resolve(process.argv[2]);await mkdir(dirname(output),{recursive:true});await writeFile(output,json,{flag:'wx',mode:0o600});}process.stdout.write(json);
}finally{await browser?.close();child.kill('SIGTERM');await new Promise(resolve=>child.exitCode!==null?resolve():child.once('exit',resolve));await rm(temporary,{recursive:true,force:true});}
