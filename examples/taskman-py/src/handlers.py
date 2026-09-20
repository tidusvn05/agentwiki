"""Command handlers."""
from .models import Task
from .storage import Storage


class Handler:
    """Dispatches CLI commands to storage operations."""

    def __init__(self) -> None:
        self.storage = Storage()

    def dispatch(self) -> None:
        self.storage.insert(Task(title="demo", done=False))
