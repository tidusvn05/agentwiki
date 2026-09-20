"""SQLite storage layer."""
from .core.db import connect
from .models import Task


class Storage:
    """Persists tasks to SQLite."""

    def __init__(self) -> None:
        self.conn = connect()

    def insert(self, task: Task) -> Task:
        self.conn.execute(
            "INSERT INTO tasks (title, done) VALUES (?, ?)",
            (task.title, int(task.done)),
        )
        return task
