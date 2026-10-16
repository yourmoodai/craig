import concurrent.futures
import tempfile
import unittest

from budget import BudgetError, Ledger


class BudgetTests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.path = directory.name+'/ledger.db'
        self.ledger = Ledger(self.path)

    def test_atomic_parallel_reservations_and_restart(self):
        def reserve(index):
            try:
                Ledger(self.path).reserve(0, 3000000, 'review', str(index))
                return True
            except BudgetError:
                return False
        with concurrent.futures.ThreadPoolExecutor(max_workers=3) as executor:
            results = list(executor.map(reserve,range(3)))
        self.assertEqual(sum(results),1)
        self.assertEqual(Ledger(self.path).summary()['committed_usd'],5.55)

    def test_qualification_has_separate_shared_ceiling(self):
        self.ledger.reserve(1500000,0,'qualification','a')
        self.ledger.reserve(1500000,0,'qualification','b')
        with self.assertRaises(BudgetError):
            self.ledger.reserve(1500000,0,'qualification','c')
        self.ledger.reserve(1500000,0,'review','d')

    def test_missing_usage_retains_reservation(self):
        key = self.ledger.reserve(200,4096,'review','a')
        before = self.ledger.summary()['committed_usd']
        with self.assertRaises(BudgetError):
            self.ledger.reconcile(key,{},1)
        self.assertEqual(self.ledger.summary()['committed_usd'],before)
        self.ledger.reconcile(key,{'usage':{'inputTokens':200,'outputTokens':30},'stopReason':'max_tokens'},1)
        self.assertLess(self.ledger.summary()['committed_usd'],before)
        with self.assertRaises(BudgetError):
            self.ledger.reconcile(key,{'usage':{'inputTokens':200,'outputTokens':30}},1)

    def test_provider_overrun_halts_resumed_calls(self):
        key = self.ledger.reserve(1,1,'review','a')
        with self.assertRaises(BudgetError):
            self.ledger.reconcile(key,{'usage':{'inputTokens':2,'outputTokens':2}},1)
        with self.assertRaisesRegex(BudgetError,'halted'):
            Ledger(self.path).reserve(1,1,'review','b')

    def test_preflight_reservation_survives_restart_once(self):
        self.ledger.record_preflight()
        Ledger(self.path).record_preflight()
        self.assertEqual(self.ledger.summary()['committed_usd'], 0.03)
        self.assertEqual(len(self.ledger.summary()['calls']), 1)

    def test_external_probe_charges_are_cumulative_and_idempotent(self):
        self.ledger.record_external_reservation(100_000_000)
        Ledger(self.path).record_external_reservation(100_000_000)
        self.ledger.record_external_reservation(50_000_000)
        self.assertEqual(self.ledger.summary()['committed_usd'], 0.1)
        self.ledger.record_external_reservation(200_000_000)
        self.assertEqual(self.ledger.summary()['committed_usd'], 0.2)
        with self.assertRaises(BudgetError):
            self.ledger.reserve(3_000_000,0,'qualification','over ceiling')
        for invalid in (-1, True, 10_000_000_001):
            with self.assertRaises(BudgetError):
                self.ledger.record_external_reservation(invalid)

    def test_merge_preserves_forked_calls_and_cumulative_external_spending(self):
        other = Ledger(self.path+'.other')
        self.ledger.record_preflight()
        other.record_preflight()
        self.ledger.record_external_reservation(100)
        other.record_external_reservation(200)
        self.ledger.reserve(10,10,'qualification','first')
        other.reserve(20,20,'qualification','second')
        before = self.ledger.summary()['committed_usd']
        self.ledger.merge(other.path)
        merged = self.ledger.summary()
        self.assertEqual(len(merged['calls']),4)
        self.assertGreater(merged['committed_usd'],before)
        self.ledger.merge(other.path)
        self.assertEqual(self.ledger.summary(),merged)

    def test_conflicting_history_cannot_replace_an_existing_charge(self):
        other = Ledger(self.path+'.other')
        key = self.ledger.reserve(20,20,'review','same-call')
        self.ledger.reconcile(key,{'usage':{'inputTokens':10,'outputTokens':10}},1)
        other.merge(self.path)
        with other.connect() as db:
            db.execute('UPDATE calls SET charged=charged+1 WHERE id=?',(key,))
        before = self.ledger.summary()
        with self.assertRaisesRegex(BudgetError,'conflicting'):
            self.ledger.merge(other.path)
        self.assertEqual(self.ledger.summary(),before)

    def test_recovery_preserves_prior_spending_and_is_idempotent(self):
        self.ledger.record_preflight()
        self.ledger.record_recovery('cancelled-1', 520676980)
        before = self.ledger.summary()
        self.ledger.record_recovery('cancelled-1', 520676980)
        self.assertEqual(self.ledger.summary(), before)
        self.assertAlmostEqual(before['committed_usd'], .550676980)
        with self.assertRaises(BudgetError):
            self.ledger.record_recovery('cancelled-1', 1)
        with self.assertRaises(BudgetError):
            self.ledger.record_recovery('other', 0)
