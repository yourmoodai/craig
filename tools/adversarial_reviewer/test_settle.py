import tempfile
from pathlib import Path
import unittest

from budget import Ledger
from settle import settle


class SettlementTests(unittest.TestCase):
    def test_interruption_preserves_charges_and_pending_reservations(self):
        with tempfile.TemporaryDirectory() as directory:
            ledger = Ledger(Path(directory) / 'ledger.db')
            charged = ledger.reserve(100, 100, 'review', 'finished')
            ledger.reconcile(charged, {'usage': {'inputTokens': 20, 'outputTokens': 10}}, 1)
            ledger.reserve(100, 100, 'review', 'interrupted')
            expected = ledger.summary()
            self.assertFalse((Path(directory) / 'settled.json').exists())
            self.assertEqual(settle(directory), expected)
            self.assertEqual(settle(directory), expected)
            self.assertEqual(Ledger(Path(directory) / 'recovery/ledger.db').summary(), expected)

    def test_missing_ledger_cannot_become_zero_spending(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, 'missing ledger'):
                settle(directory)
            self.assertFalse((Path(directory) / 'recovery/settled.json').exists())
