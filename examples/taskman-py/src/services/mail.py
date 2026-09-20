"""Mail service: sends task reminders."""
from ..core.db import connect
from ..core.naming import slugify


def send_reminder(title: str) -> str:
    conn = connect()
    return slugify(title) + str(conn is not None)
