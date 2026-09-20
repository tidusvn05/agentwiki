"""Low-level DB connection helpers."""
import sqlite3


def connect(path: str = "./tasks.db") -> sqlite3.Connection:
    return sqlite3.connect(path)
