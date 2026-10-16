"""Scoped prompt variants for bounded reviewer experiments."""
from contextlib import contextmanager
from contextvars import ContextVar

settings = ContextVar('review_settings', default=('candidate', 0, None))

@contextmanager
def select(name, temperature=0, before_call=None):
    if name not in ('current', 'candidate') or temperature not in (0, 0.7):
        raise ValueError('unsupported experiment settings')
    token = settings.set((name, temperature, before_call))
    try:
        yield
    finally:
        settings.reset(token)


def prompts(system, verification):
    if settings.get()[0] == 'current':
        return system, verification
    precision = '''For each finding, provide one concrete input and compare the old and
new outcomes. Ground language and runtime claims in supplied execution evidence
when available. Do not introduce speculative exceptions or behavior from other
languages. If uncertain, narrow the claim to the demonstrated case.
For clean changes use one sentence of at most 25 words. Keep findings concise. Every sentence must support the disposition
or explain a specific uncertainty.'''
    return system + '\n\n' + precision, verification + '\n\n' + precision + '''
Check the explanation as well as the verdict. Remove unsupported claims, even when
the underlying finding is valid.'''
