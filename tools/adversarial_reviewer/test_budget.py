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
        self.assertEqual(Ledger(self.path).summary()['committed_usd'],1.2)

    def test_qualification_has_separate_shared_ceiling(self):
        self.ledger.reserve(2000000,0,'qualification','a')
        self.ledger.reserve(2000000,0,'qualification','b')
        with self.assertRaises(BudgetError):
            self.ledger.reserve(2000000,0,'qualification','c')
        self.ledger.reserve(2000000,0,'review','d')

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
        for invalid in (-1, True, 2_000_000_001):
            with self.assertRaises(BudgetError):
                self.ledger.record_external_reservation(invalid)
