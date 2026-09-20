"""Command-line interface."""
from .handlers import Handler


def run() -> None:
    handler = Handler()
    handler.dispatch()
