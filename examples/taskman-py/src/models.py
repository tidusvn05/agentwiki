"""Domain models."""
from dataclasses import dataclass


@dataclass
class Task:
    """A single task row."""

    title: str
    done: bool
