"""Entry point: CLI task manager demo."""
import argparse

from .api import TaskAPI
from .storage import Storage


def main() -> None:
    parser = argparse.ArgumentParser(prog="taskman")
    parser.add_argument("command", choices=["add", "list", "done"])
    parser.add_argument("title", nargs="?", default="")
    args = parser.parse_args()

    api = TaskAPI(Storage("./tasks.db"))
    if args.command == "add":
        api.add(args.title)
    elif args.command == "list":
        for t in api.list():
            print(t)
    elif args.command == "done":
        api.done(args.title)


if __name__ == "__main__":
    main()
