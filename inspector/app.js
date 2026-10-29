const token=location.hash.slice(1); history.replaceState(null,'','/');
const $=id=>document.getElementById(id); let rows=[]; let before=Number.MAX_SAFE_INTEGER; let after_id='';
async function api(path,body){const response=await fetch('/api/'+path,{method:body?'POST':'GET',headers:{Authorization:'Bearer '+token,...(body?{'Content-Type':'application/json'}:{})},...(body?{body:JSON.stringify(body)}:{}),cache:'no-store'});const value=await response.json();if(!response.ok)throw Error(value.error);return value;}
function element(tag,text){const e=document.createElement(tag);if(text!==undefined)e.textContent=text;return e;}
function original_diff(row){const pre=element('pre');pre.className='original';if(row.original===null){pre.textContent='Text not recorded';return pre;}const bytes=new TextEncoder().encode(row.original);const decode=b=>new TextDecoder('utf-8',{fatal:true}).decode(b);let cursor=0;for(const span of row.decision?.applied??[]){pre.append(document.createTextNode(decode(bytes.slice(cursor,span.start))),element('mark',decode(bytes.slice(span.start,span.end))));cursor=span.end;}pre.append(document.createTextNode(decode(bytes.slice(cursor))));return pre;}
function render(){ $('events').replaceChildren(); const needle=$('search').value.toLowerCase();
 for(const row of rows){if(needle&&!JSON.stringify(row).toLowerCase().includes(needle))continue;
  const card=element('article');card.append(element('h2',new Date(row.timestamp_ms).toLocaleString()+' · '+row.outcome),element('p',row.disposition+' · '+(row.code==='adapter_failed'?'timing unavailable':row.elapsed_ms.toFixed(1)+' ms')));
  const diff=element('div');diff.className='diff';diff.append(original_diff(row));const output=element('pre',row.output??'Text not recorded');output.className='output';diff.append(output);card.append(diff);
  const details=element('details');details.append(element('summary','Span decisions and provenance'),element('pre',JSON.stringify({...row,original:undefined,output:undefined,annotation:undefined},null,2)));card.append(details);
  const annotation=row.annotation??{profanity:'unresolved',targeted_abuse:'unresolved',preservation:'unresolved',note:''};const controls={};
  for(const key of ['profanity','targeted_abuse','preservation']){const label=element('label',key.replaceAll('_',' ')+' ');const select=element('select');for(const value of ['unresolved','pass','fail','not_applicable']){const option=element('option',value);option.value=value;select.append(option);}select.value=annotation[key];controls[key]=select;label.append(select);card.append(label);}
  const note=element('textarea');note.value=annotation.note;note.maxLength=4096;note.setAttribute('aria-label','Reviewer note');card.append(note);
  const save=element('button','Save human judgment');save.onclick=()=>run(async()=>{await api('annotate',{id:row.id,annotation:{...Object.fromEntries(Object.entries(controls).map(([k,v])=>[k,v.value])),note:note.value}});save.textContent='Saved';});card.append(save);$('events').append(card);
 }
}
async function refresh(){const state=await api('status');$('status').textContent=`${state.events} events · full-text recording ${state.full_text?'on':'off'} · no remote telemetry`;rows=await api('events',{before,after_id,outcome:$('outcome').value});render();}
async function run(action){try{await action();}catch(error){$('status').textContent='Inspection unavailable: '+error.message;}}
$('refresh').onclick=()=>run(()=>{before=Number.MAX_SAFE_INTEGER;after_id='';return refresh();});$('outcome').onchange=$('refresh').onclick;$('search').oninput=render;
$('older').onclick=()=>run(()=>{if(rows.length){before=rows.at(-1).timestamp_ms;after_id=rows.at(-1).id;}return refresh();});
$('enable').onclick=()=>run(async()=>{if(confirm('Save original prompts and replacements locally? They may contain secrets.')){await api('consent',{enabled:true});await refresh();}});
$('disable').onclick=()=>run(async()=>{await api('consent',{enabled:false});await refresh();});
$('export').onclick=()=>run(async()=>{if(!confirm('Export potentially sensitive prompts? This copy will not expire or be deleted by Savior.'))return;const response=await fetch('/api/export',{headers:{Authorization:'Bearer '+token},cache:'no-store'});if(!response.ok)throw Error('export_failed');const url=URL.createObjectURL(await response.blob());const link=element('a');link.href=url;link.download='savior-replacements.parquet';link.click();setTimeout(()=>URL.revokeObjectURL(url),1000);});
$('delete').onclick=()=>run(async()=>{if(confirm('Delete retained records and annotations, and disable recording? Exported copies are unaffected.')){await api('delete',{confirm:true});await refresh();}});
run(refresh);
