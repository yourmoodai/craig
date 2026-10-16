"""Capture synthetic qualification responses without retaining PR prompts."""
from contextlib import contextmanager
from contextvars import ContextVar

_events = ContextVar('qualification_events', default=None)


@contextmanager
def capture():
    events = []
    token = _events.set(events)
    try:
        yield events
    finally:
        _events.reset(token)


def record(**event):
    events = _events.get()
    if events is not None:
        events.append(event)
