"""Naming helpers."""


def slugify(name: str) -> str:
    return name.strip().lower().replace(" ", "-")
