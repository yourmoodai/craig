"""Durable reservations prevent concurrent or resumed calls from reusing dollars."""
from contextlib import contextmanager
import json
import sqlite3
import time
import uuid

LIMIT = 2_000_000_000  # nanodollars: $2 pilot, including uncertain calls
QUALIFICATION_LIMIT = 400_000_000  # nanodollars: $0.40
INPUT_RATE = 1100  # nanodollars per token; US Haiku 4.5 on-demand
OUTPUT_RATE = 5500


class BudgetError(Exception):
    pass


def cost(inputs, outputs):
    return inputs * INPUT_RATE + outputs * OUTPUT_RATE


class Ledger:
    def __init__(self, path):
        self.path = path
        with self.connect() as db:
            db.execute('CREATE TABLE IF NOT EXISTS calls (id TEXT PRIMARY KEY, phase TEXT, identity TEXT, reserved INTEGER, charged INTEGER, metrics TEXT)')

    @contextmanager
    def connect(self):
        db = sqlite3.connect(self.path, timeout=30)
        try:
            with db:
                yield db
        finally:
            db.close()

    def reserve(self, inputs, outputs, phase, identity):
        if phase not in ('qualification', 'review') or any(type(n) is not int or n < 0 for n in (inputs, outputs)):
            raise BudgetError('invalid reservation parameters')
        amount = cost(inputs, outputs)
        with self.connect() as db:
            db.execute('BEGIN IMMEDIATE')
            if db.execute('SELECT COUNT(*) FROM calls WHERE charged > reserved').fetchone()[0]:
                raise BudgetError('provider usage exceeded a reservation; pilot halted')
            total = db.execute('SELECT COALESCE(SUM(COALESCE(charged,reserved)),0) FROM calls').fetchone()[0]
            qualification = db.execute("SELECT COALESCE(SUM(COALESCE(charged,reserved)),0) FROM calls WHERE phase='qualification'").fetchone()[0]
            if total + amount > LIMIT or (phase == 'qualification' and qualification + amount > QUALIFICATION_LIMIT):
                raise BudgetError('pilot budget exhausted; assessment remains incomplete')
            key = uuid.uuid4().hex
            db.execute('INSERT INTO calls VALUES (?,?,?,?,NULL,NULL)', (key, phase, identity, amount))
        return key

    def reconcile(self, key, envelope, elapsed):
        usage = envelope.get('usage', {})
        inputs, outputs = usage.get('inputTokens'), usage.get('outputTokens')
        if any(type(n) is not int or n < 0 for n in (inputs, outputs)):
            raise BudgetError('missing usage; full reservation retained')
        if usage.get('cacheReadInputTokens', 0) or usage.get('cacheWriteInputTokens', 0):
            raise BudgetError('unexpected cache usage; full reservation retained')
        metrics = {'usage': usage, 'stop_reason': envelope.get('stopReason'),
                   'latency_seconds': elapsed, 'recorded_at': time.time()}
        with self.connect() as db:
            db.execute('BEGIN IMMEDIATE')
            row = db.execute('SELECT reserved,charged FROM calls WHERE id=?', (key,)).fetchone()
            if row is None or row[1] is not None:
                raise BudgetError('invalid reservation')
            actual = cost(inputs, outputs)
            db.execute('UPDATE calls SET charged=?,metrics=? WHERE id=?', (actual, json.dumps(metrics), key))
        if actual > row[0]:
            raise BudgetError('provider usage exceeded reservation; stop review')

    def summary(self):
        with self.connect() as db:
            rows = db.execute('SELECT id,phase,identity,reserved,charged,metrics FROM calls ORDER BY rowid').fetchall()
        return {'limit_usd': LIMIT / 1e9, 'committed_usd': sum(r[4] if r[4] is not None else r[3] for r in rows) / 1e9,
                'calls': [dict(id=r[0], phase=r[1], identity=r[2], reserved_usd=r[3]/1e9,
                               actual_usd=None if r[4] is None else r[4]/1e9,
                               metrics=json.loads(r[5]) if r[5] else None) for r in rows]}
