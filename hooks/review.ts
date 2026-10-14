import type {LanguageAssessment} from '../src/protocol';

export interface ReviewHost {
  run(request: unknown): Promise<{exitCode: number; stdout: string}>;
  session(): Promise<string>;
  now(): Promise<number>;
  enabled(): Promise<unknown>;
  save(enabled: boolean): Promise<unknown>;
  invalidate(): void;
}
type Consent={live:boolean;history:boolean;generation:number};
export class PersonalReview {
  consent:Consent={live:false,history:false,generation:0};
  view:'inspection'|'metrics'|'policy'='inspection';
  days=3;
  now=0;
  message='personal metrics are off; no history is read until enabled';
  data:Record<string,unknown>={};
  scanning=false;
  session='';
  private context:string[]=[];
  private serial=0;
  private epoch=0;
  summary(source:'live'|'history'):string {
    const rows=(this.data.summary as Record<string,Record<string,number>>|undefined)?.[source];
    if(!rows)return `${source}: no metrics loaded`;
    return `${source}: ${rows.observations} prompts; ${rows.assessed} assessed; ${rows.replacement_count} replacements; ${rows.removed_characters} characters removed; ${rows.judge_complete} judged; ${rows.judge_unresolved} unresolved; ${rows.assessment_failed} assessment failures`;
  }
  categories(source:'live'|'history'):string {
    const rows=(this.data.summary as Record<string,{before_categories?:Record<string,number>}>|undefined)?.[source]?.before_categories??{};
    return `judged original language: ${rows.none??0} without a concern; ${rows.general_profanity??0} general profanity; ${rows.directed_abuse??0} directed abuse; ${rows.uncertain??0} uncertain`;
  }
  async request(host:ReviewHost,request:unknown):Promise<Record<string,any>> {
    const answer=await host.run(request);
    if(answer.exitCode!==0) throw Error('review_unavailable');
    const result=JSON.parse(answer.stdout);
    if(result.version!==1||result.status!=='ok'||!result.value||typeof result.value!=='object') throw Error('review_unavailable');
    return result.value;
  }
  async start(host:ReviewHost) {
    this.epoch++;this.context=[];this.session=await host.session();this.now=await host.now();
    if(await host.enabled()===true) {
      try{await this.refresh(host);}catch{this.consent={live:false,history:false,generation:0};this.message='personal metrics unavailable; rewriting remains active';}
    }
  }
  async end(host:ReviewHost) {
    this.epoch++;this.context=[];
    if(this.consent.live||this.consent.history) {try{await this.request(host,{command:'cancel',session:this.session});}catch{}}
  }
  async refresh(host:ReviewHost) {
    this.data=await this.request(host,{command:'status'});
    const c=this.data.consent as Consent;
    if(typeof c?.live!=='boolean'||typeof c?.history!=='boolean'||!Number.isSafeInteger(c?.generation))throw Error('invalid_review_response');
    this.consent=c;this.now=await host.now();
    this.message='local estimates only; classifier confidence is not abuse severity or provider approval';
  }
  async toggle(host:ReviewHost,field:'live'|'history') {
    try {
      this.epoch++;this.context=[];
      const next={...this.consent,[field]:!this.consent[field]};
      const result=await this.request(host,{command:'consent',live:next.live,history:next.history});
      this.consent=result.consent;
      await host.save(this.consent.live||this.consent.history);
      await this.refresh(host);
    }catch{this.message='personal metrics setting could not be changed';}
    host.invalidate();
  }
  async observe(host:ReviewHost,original:string,replacement:string|null,assessment:LanguageAssessment) {
    if(!this.consent.live)return;
    const epoch=this.epoch;
    const timestamp=await host.now();
    const observation={id:`live-${timestamp}-${++this.serial}`,session:this.session,generation:this.consent.generation,timestamp_ms:timestamp,source:'live',original,replacement,assessment,context:this.context,authorship_verified:true};
    try{await this.request(host,{command:'observe',observation});}catch{this.message='personal metrics unavailable; sanitized submission is unchanged';}
    if(epoch!==this.epoch)return;
    if(replacement!==null)this.context=[...this.context,replacement].slice(-4);
    while(new TextEncoder().encode(this.context.join('')).length>16384)this.context.shift(); // 16 KiB
  }
  async scan(host:ReviewHost) {
    if(!this.consent.history||this.scanning)return;
    this.scanning=true;this.message='scanning stored user messages in the selected date window';host.invalidate();
    const epoch=this.epoch;
    try {const result=await this.request(host,{command:'scan',days:this.days});if(epoch===this.epoch){await this.refresh(host);this.message=JSON.stringify(result);}}
    catch{if(epoch===this.epoch)this.message='history scan incomplete; coverage is not a clean result';}
    finally{this.scanning=false;host.invalidate();}
  }
  async cancel(host:ReviewHost) {
    this.epoch++;this.context=[];
    try {const result=await this.request(host,{command:'consent',live:this.consent.live,history:this.consent.history});this.consent=result.consent;this.message='queued review cancelled';}catch{this.message='review cancellation unavailable';}
    host.invalidate();
  }
  async clear(host:ReviewHost) {
    this.epoch++;this.context=[];
    try{await this.request(host,{command:'delete'});await this.refresh(host);this.message='personal metrics deleted';}catch{this.message='metrics deletion unavailable';}
    host.invalidate();
  }
  async export(host:ReviewHost) {
    try{const result=await this.request(host,{command:'export_file'});this.message=`text-free metrics exported: ${result.path}`;}catch{this.message='metrics export unavailable';}
    host.invalidate();
  }
}
