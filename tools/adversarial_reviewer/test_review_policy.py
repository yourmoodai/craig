"""Exercise the actual workflow publisher against GitHub-shaped responses."""
import json
from pathlib import Path
import shutil
import subprocess
import textwrap
import unittest

ROOT=Path(__file__).resolve().parents[2]
@unittest.skipUnless(shutil.which('node'),'node required')
class ReviewPolicyTests(unittest.TestCase):
    def publish(self, **changes):
        report={'commit_id':'head','qualified':True,'complete':True,'disposition':'APPROVE','findings':[],'body':'Reviewed all changes.'}
        report.update(changes.pop('report',{}))
        case={'report':report,'head':'head','reviews':[],**changes}
        policy=textwrap.dedent((ROOT/'.github/workflows/adversarial-review.yml').read_text().rsplit('script: |\n',1)[1])
        harness='''
const input=JSON.parse(process.argv[1]); const calls=[];
process.env.EXPECTED_HEAD='head';process.env.PR_NUMBER='1';
const context={repo:{owner:'yourmoodai',repo:'craig'}};
const core={notice:()=>{}};
const require=()=>({readFileSync:()=>JSON.stringify(input.report)});
const github={rest:{pulls:{get:async()=>({data:{state:'open',head:{sha:input.head}}}),listReviews:()=>{},createReview:async r=>calls.push(r)}},paginate:async()=>input.reviews};
'''
        result=subprocess.run(['node','-e',harness+'\n(async()=>{'+policy+'\n})().then(()=>console.log(JSON.stringify(calls)));',json.dumps(case)],capture_output=True,text=True,timeout=10)
        self.assertEqual(result.returncode,0,result.stderr)
        return json.loads(result.stdout)
    def test_clean_current_review_approves(self):
        self.assertEqual(self.publish()[0]['event'],'APPROVE')
    def test_findings_and_incomplete_reviews_only_comment(self):
        for patch in [{'findings':[{'title':'defect'}]},{'complete':False},{'qualified':False},{'disposition':'REQUEST_CHANGES'},{'disposition':'COMMENT'}]:
            self.assertEqual(self.publish(report=patch)[0]['event'],'COMMENT')
    def test_stale_review_is_discarded(self):
        self.assertEqual(self.publish(head='new'),[])
        self.assertEqual(self.publish(report={'commit_id':'old'}),[])
    def test_duplicate_review_is_not_published(self):
        review={'user':{'login':'github-actions[bot]'},'commit_id':'head','state':'APPROVED','body':'<!-- craig-adversarial-review -->'}
        self.assertEqual(self.publish(reviews=[review]),[])
