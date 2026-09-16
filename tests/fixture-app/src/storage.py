"""SQLite storage layer."""
import sqlite3

from .models import Task

SCHEMA = """
CREATE TABLE IF NOT EXISTS tasks (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT NOT NULL,
    done INTEGER NOT NULL DEFAULT 0
);
"""


class Storage:
    """Persists tasks to SQLite."""

    def __init__(self, path: str) -> None:
        self.conn = sqlite3.connect(path)
        self.conn.execute(SCHEMA)

    def insert(self, task: Task) -> Task:
        cur = self.conn.execute(
            "INSERT INTO tasks (title, done) VALUES (?, ?)",
            (task.title, int(task.done)),
        )
        task.id = cur.lastrowid
        self.conn.commit()
        return task

    def all(self) -> list[Task]:
        rows = self.conn.execute(
            "SELECT id, title, done FROM tasks ORDER BY id"
        ).fetchall()
        return [Task(id=r[0], title=r[1], done=bool(r[2])) for r in rows]

    def mark_done(self, title: str) -> None:
        self.conn.execute(
            "UPDATE tasks SET done = 1 WHERE title = ?", (title,)
        )
        self.conn.commit()
