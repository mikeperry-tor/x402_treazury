"""Tiny JSON-Schema builders for hand-written tool input schemas."""

from __future__ import annotations

from typing import Any


def s(description: str, **extra: Any) -> dict:
    return {"type": "string", "description": description, **extra}


def i(description: str, **extra: Any) -> dict:
    return {"type": "integer", "description": description, **extra}


def b(description: str, **extra: Any) -> dict:
    return {"type": "boolean", "description": description, **extra}


def arr(item_type: str, description: str, **extra: Any) -> dict:
    return {"type": "array", "items": {"type": item_type}, "description": description, **extra}


def obj(properties: dict[str, dict], required: list[str] | None = None) -> dict:
    schema: dict[str, Any] = {"type": "object", "properties": properties, "additionalProperties": False}
    if required:
        schema["required"] = required
    return schema
