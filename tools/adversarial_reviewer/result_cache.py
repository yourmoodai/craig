"""Reuse validated assessments from trusted workflow artifacts, never approvals."""
import hashlib
import json
import os
from pathlib import Path
import time
import zipfile

ROOT = Path(__file__).parent
SOURCES = ('result_cache.py', 'adversarial_review.py', 'adaptive_review.py', 'chunk_plan.py',
           'bedrock_client.py', 'prompt_profiles.py', 'evidence.py', 'runtime_facts.py',
           'qualify.py', 'evaluate_model.py')


def fingerprint():
    return hashlib.sha256(b''.join((ROOT / name).read_bytes() for name in SOURCES)).hexdigest()


def key(kind, payload):
    return hashlib.sha256(json.dumps([kind, fingerprint(), payload], sort_keys=True).encode()).hexdigest()


def directory():
    path = os.environ.get('REVIEW_RESULT_CACHE')
    return Path(path) if path else None


def get(identity, ttl):
    root = directory()
    if root is None or os.environ.get('REVIEW_CACHE_DISABLED') == 'true':
        return None
    try:
        path = root / (identity + '.json')
        if path.stat().st_size > 1_000_000:  # 1 MB per cached assessment
            return None
        item = json.loads(path.read_text())
        if item['key'] != identity or not 0 <= time.time() - item['created'] <= ttl:
            return None
        return item['value']
    except (OSError, ValueError, TypeError, KeyError):
        return None


def put(identity, value):
    root = directory()
    if root is None or os.environ.get('REVIEW_CACHE_DISABLED') == 'true':
        return
    root.mkdir(parents=True, exist_ok=True)
    data = json.dumps({'key': identity, 'created': time.time(), 'value': value})
    if len(data.encode()) > 1_000_000:
        return
    temporary = root / (identity + '.tmp')
    temporary.write_text(data)
    temporary.replace(root / (identity + '.json'))


def restore(archives):
    root = directory()
    if root is None:
        return
    root.mkdir(parents=True, exist_ok=True)
    total = 0
    for archive in archives:
        try:
            with zipfile.ZipFile(archive) as zipped:
                for entry in zipped.infolist():
                    name = entry.filename
                    if len(name) != 69 or not name.endswith('.json') or any(c not in '0123456789abcdef' for c in name[:-5]):
                        continue
                    total += entry.file_size
                    if total > 20_000_000:  # 20 MB restored cache limit
                        return
                    if entry.file_size > 1_000_000:
                        continue
                    data = zipped.read(entry)
                    item = json.loads(data)
                    if item['key'] != name[:-5] or not 0 <= time.time()-item['created'] <= 7*86400:
                        continue
                    # Newest trusted artifact wins; cache corruption is a miss.
                    if not (root / name).exists():
                        (root / name).write_bytes(data)
        except (OSError, ValueError, KeyError, TypeError, zipfile.BadZipFile):
            continue


def measure(kind, **values):
    root = directory()
    if root is not None:
        root.mkdir(parents=True, exist_ok=True)
        with (root.parent / 'usage.jsonl').open('a') as output:
            output.write(json.dumps({'kind': kind, **values})+'\n')


def measurements():
    root = directory()
    result = {'cache_hits': 0, 'model_calls': 0, 'input_tokens': 0, 'output_tokens': 0}
    if root is None or not (root.parent / 'usage.jsonl').exists():
        return result
    for line in (root.parent / 'usage.jsonl').read_text().splitlines():
        event = json.loads(line)
        if event['kind'] == 'hit':
            result['cache_hits'] += 1
        if event['kind'] == 'call':
            result['model_calls'] += 1
            result['input_tokens'] += event.get('input_tokens', 0)
            result['output_tokens'] += event.get('output_tokens', 0)
    return result
