"""Sync service: pushes local tasks to a remote."""
from ..core.db import connect
from ..storage import Storage


def sync_all() -> None:
    conn = connect()
    storage = Storage()
    rows = conn.execute("SELECT id FROM tasks").fetchall()
    _ = (storage, rows)
