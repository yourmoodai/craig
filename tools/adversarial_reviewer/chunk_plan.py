"""Deterministic ownership of change blocks, independent of request packing."""
from dataclasses import dataclass, replace
import hashlib
import json
from pathlib import PurePosixPath
import re

from budget import cost

HUNK = re.compile(r'^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@')


@dataclass(frozen=True)
class Unit:
    id: str
    path: str
    header: str
    rows: tuple
    old: int = 0
    new: int = 0
    start: int = 0
    metadata: bool = False

    def render(self):
        if self.metadata:
            return self.header
        old_count = sum(row.startswith((' ', '-')) for row in self.rows)
        new_count = sum(row.startswith((' ', '+')) for row in self.rows)
        old_start = self.old if old_count else max(0, self.old-1)
        new_start = self.new if new_count else max(0, self.new-1)
        return f'{self.header}@@ -{old_start},{old_count} +{new_start},{new_count} @@\n'+''.join(self.rows)

    def anchors(self):
        if self.metadata:
            return [{'id': self.id, 'file': self.path, 'line': 0, 'side': 'file', 'evidence': self.header.strip()}]
        old, new = self.old, self.new
        anchors = []
        for offset, row in enumerate(self.rows, self.start):
            if row.startswith((' ', '+', '-')):
                side = 'old' if row.startswith('-') else 'new'
                anchors.append({'id': f'{self.id.split("/")[0]}:{offset}', 'file': self.path,
                                'line': old if side == 'old' else new, 'side': side, 'evidence': row[1:].rstrip('\n')})
            old += row.startswith((' ', '-'))
            new += row.startswith((' ', '+'))
        return anchors

    def split(self):
        if self.metadata or len(self.rows) < 2:
            raise ValueError('indivisible change exceeds review capacity')
        midpoint = len(self.rows)//2
        # A no-newline marker belongs to its preceding source line.
        if self.rows[midpoint].startswith('\\'):
            midpoint += 1
        if midpoint == len(self.rows):
            raise ValueError('indivisible change exceeds review capacity')
        left, right = self.rows[:midpoint], self.rows[midpoint:]
        return [replace(self, id=self.id+'/0', rows=left),
                replace(self, id=self.id+'/1', rows=right, start=self.start+midpoint,
                        old=self.old+sum(r.startswith((' ', '-')) for r in left),
                        new=self.new+sum(r.startswith((' ', '+')) for r in left))]


def parse(diff):
    units = []
    for section in re.split(r'(?=^diff --git )', diff, flags=re.M):
        if not section.strip():
            continue
        lines = section.splitlines(keepends=True)
        if not lines[0].startswith('diff --git '):
            raise ValueError('diff is missing file headers')
        if re.search(r'^(Binary files .* differ|GIT binary patch)$', section, re.M):
            raise ValueError('unsupported binary coverage')
        paths = re.findall(r'^\+\+\+ b/(.*?)(?:\t.*)?$', section, re.M)
        if not paths:
            paths = re.findall(r'^--- a/(.*?)(?:\t.*)?$', section, re.M)
        if not paths:
            paths = re.findall(r'^rename to (.+)$', section, re.M)
        path = paths[0] if paths else lines[0].rstrip().split(' b/', 1)[-1]
        hunks = [i for i, row in enumerate(lines) if HUNK.match(row)]
        header = ''.join(lines[:hunks[0]]) if hunks else section
        prefix = hashlib.sha256(lines[0].encode()).hexdigest()[:16]
        if not hunks:
            units.append(Unit(prefix+':file', path, header, (), metadata=True))
        for index, begin in enumerate(hunks):
            end = hunks[index+1] if index+1 < len(hunks) else len(lines)
            match = HUNK.match(lines[begin])
            old = int(match[1]) + (match[2] == '0')
            new = int(match[3]) + (match[4] == '0')
            rows = lines[begin+1:end]
            starts = [i for i, row in enumerate(rows) if row.startswith(('+', '-')) and (i == 0 or not rows[i-1].startswith(('+', '-')))]
            if not starts:
                raise ValueError('hunk contains no reviewable change')
            # Context belongs to one unit, so every source row has one primary owner.
            boundaries = [0, *starts[1:], len(rows)]
            for left, right in zip(boundaries, boundaries[1:]):
                unit_id = f'{prefix}:{begin+left}'
                units.append(Unit(unit_id, path, header, tuple(rows[left:right]), old, new, begin+1+left))
                old += sum(r.startswith((' ', '-')) for r in rows[left:right])
                new += sum(r.startswith((' ', '+')) for r in rows[left:right])
    return units


def ceiling(count):
    return 4000 if count <= 8 else 8000 if count <= 32 else 16000


def render(units):
    return ''.join(unit.render() for unit in units)


def anchors(units):
    return [anchor for unit in units for anchor in unit.anchors() if anchor['evidence'].strip()]


def related(left, right):
    if left.path == right.path:
        return 0
    if PurePosixPath(left.path).name in ''.join(right.rows) or PurePosixPath(right.path).name in ''.join(left.rows):
        return 1
    if PurePosixPath(left.path).parent == PurePosixPath(right.path).parent:
        for source, dependency in ((left, right), (right, left)):
            imports = re.findall(r'^[ +](?:from|import) ([a-zA-Z_]\w*)', ''.join(source.rows), re.M)
            if PurePosixPath(dependency.path).suffix == '.py' and PurePosixPath(dependency.path).stem in imports:
                return 1
        return 2
    return 3


def plan(diff, count, messages, *, strategy='adaptive'):
    original = parse(diff)
    target = ceiling(len(original))
    pending = sorted(original, key=lambda u: (u.path, u.start))
    chunks = []
    cache = {}

    def tokens(text):
        if text not in cache:
            cache[text] = count([{'role': 'user', 'content': text}])
        return cache[text]

    def fits(group):
        text = render(group)
        if strategy == 'baseline':
            return len(text.encode()) <= 49152  # 48 KiB baseline
        # This estimate only packs candidates; CountTokens enforces the actual ceiling.
        return len(group) <= 16 and len({u.path for u in group}) <= 8 and len(text.encode()) <= target*4

    while pending:
        group = [pending.pop(0)]
        if not fits(group):
            pending[0:0] = group[0].split()
            continue
        while pending:
            candidates = sorted(range(len(pending)), key=lambda i: (min(related(u, pending[i]) for u in group), pending[i].path, pending[i].start))
            selected = next((i for i in candidates if fits(group+[pending[i]])), None)
            if selected is None:
                break
            group.append(pending.pop(selected))
        chunks.append(group)

    planned = []
    while chunks:
        group = chunks.pop(0)
        if strategy == 'adaptive' and tokens(render(group)) > target:
            chunks[0:0] = subdivide(group)
            continue
        support, omitted = [], []
        primary_ids = {u.id.split('/')[0] for u in group}
        for unit in original:
            if unit.id in primary_ids or not any(related(u, unit) <= 1 for u in group):
                continue
            if len(render(support+[unit]).encode()) <= 8000:
                support.append(unit)
            else:
                omitted.append(unit.id)
        prompt = messages(group, render(support))
        input_tokens = count(prompt)
        if input_tokens > 48000 or sum(len(m['content'].encode()) for m in prompt) > 131072:
            if support:
                omitted.extend(u.id for u in support)
                support = []
                prompt = messages(group, '')
                input_tokens = count(prompt)
            if input_tokens > 48000 or sum(len(m['content'].encode()) for m in prompt) > 131072:
                children = subdivide(group)
                chunks[0:0] = children
                continue
        planned.append({'units': group, 'support': render(support), 'input_tokens': input_tokens,
                        'prompt_sha256': hashlib.sha256(json.dumps(prompt, sort_keys=True).encode()).hexdigest(),
                        'support_tokens': tokens(render(support)) if support else 0,
                        'omitted_support': omitted})
    return {'strategy': strategy, 'original_units': len(original), 'primary_token_ceiling': target,
            'chunks': planned}


def subdivide(group):
    if len(group) == 1:
        return [[u] for u in group[0].split()]
    middle = len(group)//2
    return [group[:middle], group[middle:]]


def manifest(planned):
    chunks = [{'index': i, 'unit_ids': [u.id for u in c['units']],
               'files': sorted({u.path for u in c['units']}),
               'evidence_ids': [a['id'] for a in anchors(c['units'])],
               **{k: c[k] for k in ('input_tokens', 'support_tokens', 'omitted_support', 'prompt_sha256')},
               'maximum_assessment_usd': cost(c['input_tokens'], 4096)/1e9}
              for i, c in enumerate(planned['chunks'], 1)]
    return {**{k: v for k, v in planned.items() if k != 'chunks'}, 'chunks': chunks,
            'maximum_first_pass_usd': sum(c['maximum_assessment_usd'] for c in chunks),
            'note': 'verification, subdivision and synthesis require additional reservations'}
