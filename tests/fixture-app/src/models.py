"""Domain models."""
from dataclasses import dataclass


@dataclass
class Task:
    """A single task row."""

    id: int | None
    title: str
    done: bool
