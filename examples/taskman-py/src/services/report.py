"""Report service: renders task reports."""
from ..core.naming import slugify
from ..models import Task


def render(tasks: list[Task]) -> str:
    return "\n".join(slugify(t.title) for t in tasks)
