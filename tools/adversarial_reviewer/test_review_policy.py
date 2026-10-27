"""Exercise the comment publisher against GitHub-shaped responses."""
import json
from pathlib import Path
import shutil
import subprocess
import unittest

ROOT=Path(__file__).resolve().parents[2]
@unittest.skipUnless(shutil.which('node'),'node required')
class ReviewPolicyTests(unittest.TestCase):
    def publish(self, **changes):
        report={'test_evidence':{'head':'head','runs':[{'workflow':'.github/workflows/'+f,'conclusion':'success'} for f in ['verify.yml','commit-messages.yml','release.yml']]},'commit_id':'head','qualified':True,'complete':True,'synthesis_complete':True,'overview':'word '*50,'review_failures':[],'disposition':'APPROVE','findings':[],'body':'**Verdict: Approved**'}
        report.update(changes.pop('report',{}))
        clean=50 <= len(report['overview'].split()) <= 100 and report['qualified'] and report['complete'] and report['synthesis_complete'] and not report['review_failures'] and report['disposition']=='APPROVE' and not report['findings']
        report['body']=changes.pop('body','**Verdict: '+('Approved' if clean else 'Request changes')+'**')
        case={'report':report,'head':'head','comments':[],'phase':'complete',**changes}
        harness='''
const input=JSON.parse(process.argv[1]); const calls=[]; const failures=[];
const context={repo:{owner:'yourmoodai',repo:'savior'},runId:42};
const core={notice:()=>{},setFailed:message=>failures.push(message)};
let reads=0;
const github={rest:{pulls:{listReviews:'reviews',createReview:async r=>calls.push({kind:'review',...r}),get:async()=>({data:{state:'open',head:{sha:input.staleAfterRead && reads++ ? 'new' : input.head}}})},issues:{listComments:'list',updateComment:async r=>calls.push({kind:'update',...r}),createComment:async r=>calls.push({kind:'create',...r})}},paginate:async method=>method==='reviews' ? (input.reviews||[]) : input.comments};
const publish=require('./tools/adversarial_reviewer/publish.cjs');
publish({github,context,core,head:'head',pr:1,report:input.report,phase:input.phase}).then(()=>console.log(JSON.stringify({calls,failures})));
'''
        result=subprocess.run(['node','--input-type=commonjs','-e',harness,json.dumps(case)],cwd=ROOT,capture_output=True,text=True,timeout=10)
        if case.get('invalid'):
            self.assertNotEqual(result.returncode,0)
            return {}
        self.assertEqual(result.returncode,0,result.stderr)
        return json.loads(result.stdout)
    def test_clean_current_review_updates_comment_and_passes(self):
        result=self.publish()
        self.assertEqual(result['calls'][0]['kind'],'create')
        self.assertIn('**Verdict: Approved**',result['calls'][0]['body'])
        self.assertEqual(result['failures'],[])
        self.assertEqual(result['calls'][1]['event'],'APPROVE')
    def test_findings_and_incomplete_reviews_fail_after_publication(self):
        for patch in [{'findings':[{'title':'defect'}]},{'complete':False},{'qualified':False},{'synthesis_complete':False},{'overview':'short'},{'review_failures':['timeout']},{'disposition':'REQUEST_CHANGES'},{'disposition':'COMMENT'}]:
            result=self.publish(report=patch)
            self.assertIn('**Verdict: Request changes**',result['calls'][0]['body'])
            self.assertEqual(len(result['failures']),1)
    def test_body_cannot_override_computed_verdict(self):
        self.publish(report={'complete':False},body='**Verdict: Approved**',invalid=True)
    def test_stale_review_is_discarded(self):
        for changes in [{'head':'new'},{'report':{'commit_id':'old'}},{'staleAfterRead':True}]:
            self.assertEqual(self.publish(**changes)['calls'],[])
    def test_repeated_assessments_keep_the_same_comment(self):
        body=self.publish()['calls'][0]['body']
        comment={'id':8,'user':{'login':'github-actions[bot]','type':'Bot'},'body':body}
        self.assertEqual(self.publish(comments=[comment],reviews=[{'user':{'login':'github-actions[bot]','type':'Bot'},'commit_id':'head','state':'APPROVED'}])['calls'],[])
        changed=self.publish(comments=[comment],report={'complete':False})
        self.assertEqual(changed['calls'][0]['kind'],'update')
        self.assertEqual(changed['calls'][0]['comment_id'],8)
    def test_foreign_markers_are_not_owned(self):
        comment={'id':8,'user':{'login':'someone','type':'User'},'body':'<!-- savior-adversarial-review -->'}
        self.assertEqual(self.publish(comments=[comment])['calls'][0]['kind'],'create')
    def test_waiting_and_failed_phases_share_the_comment(self):
        result=self.publish(phase='waiting')
        self.assertEqual(result['failures'],[])
        comment={'id':8,'user':{'login':'github-actions[bot]','type':'Bot'},'body':result['calls'][0]['body']}
        result=self.publish(phase='failed',comments=[comment])
        self.assertEqual(result['calls'][0]['comment_id'],8)
        self.assertEqual(len(result['failures']),1)

    def test_infrastructure_failure_never_requests_changes_formally(self):
        result=self.publish(report={'complete':False})
        self.assertFalse(any(c['kind']=='review' for c in result['calls']))

    def test_validated_findings_request_changes(self):
        result=self.publish(report={'findings':[{'title':'demonstrated defect'}], 'disposition':'REQUEST_CHANGES'})
        self.assertEqual(result['calls'][1]['event'],'REQUEST_CHANGES')

    def test_formal_review_is_independent_of_test_completion(self):
        for runs in ([], [{'status':'in_progress'}], [{'conclusion':'failure'}]):
            result = self.publish(report={'test_evidence':{'head':'head','runs':runs}})
            self.assertEqual(result['calls'][1]['event'], 'APPROVE')

    def test_size_policy_no_longer_exists(self):
        self.publish(phase='policy',invalid=True)
