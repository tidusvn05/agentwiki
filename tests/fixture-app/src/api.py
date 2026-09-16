"""Task API surface used by the CLI."""
from .models import Task
from .storage import Storage


class TaskAPI:
    """High-level task operations."""

    def __init__(self, storage: Storage) -> None:
        self.storage = storage

    def add(self, title: str) -> Task:
        task = Task(id=None, title=title, done=False)
        return self.storage.insert(task)

    def list(self) -> list[Task]:
        return self.storage.all()

    def done(self, title: str) -> None:
        self.storage.mark_done(title)
