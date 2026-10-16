"""Snapshot committed charges and pending reservations after an interrupted run."""
import json
from pathlib import Path
import sqlite3
import sys

from budget import Ledger


def settle(directory):
    directory = Path(directory)
    source = directory / 'ledger.db'
    if not source.is_file():
        raise ValueError('missing ledger; reservation remains unresolved')
    output = directory / 'recovery'
    output.mkdir(exist_ok=True)
    # SQLite backup includes committed WAL data without forgiving unfinished calls.
    with sqlite3.connect(source.resolve().as_uri() + '?mode=ro', uri=True) as db:
        with sqlite3.connect(output / 'ledger.db') as snapshot:
            db.backup(snapshot)
    summary = Ledger(output / 'ledger.db').summary()
    (output / 'settled.json').write_text(json.dumps(summary, indent=2) + '\n')
    return summary


if __name__ == '__main__':
    settle(sys.argv[1])
