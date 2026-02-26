"""Input validators for SlurmTerm."""

from __future__ import annotations

import re


def parse_time(time_str: str) -> int:
    """Parse a Slurm-style time string and return total seconds.

    Supported formats::

        SS
        MM:SS
        HH:MM:SS
        D-HH:MM:SS

    Raises :class:`ValueError` on invalid input.
    """
    time_str = time_str.strip()
    if not time_str:
        raise ValueError("Empty time string")

    days = 0
    if "-" in time_str:
        day_part, time_str = time_str.split("-", 1)
        days = int(day_part)

    parts = time_str.split(":")
    if len(parts) == 1:
        return days * 86400 + int(parts[0])
    elif len(parts) == 2:
        return days * 86400 + int(parts[0]) * 60 + int(parts[1])
    elif len(parts) == 3:
        return (
            days * 86400
            + int(parts[0]) * 3600
            + int(parts[1]) * 60
            + int(parts[2])
        )
    else:
        raise ValueError(f"Invalid time format: {time_str!r}")


def format_time(seconds: int) -> str:
    """Convert *seconds* to ``HH:MM:SS`` (or ``D-HH:MM:SS`` when ≥ 1 day)."""
    if seconds < 0:
        raise ValueError("Negative seconds")
    days, rem = divmod(seconds, 86400)
    hours, rem = divmod(rem, 3600)
    minutes, secs = divmod(rem, 60)
    if days:
        return f"{days}-{hours:02d}:{minutes:02d}:{secs:02d}"
    return f"{hours:02d}:{minutes:02d}:{secs:02d}"


_MEM_RE = re.compile(r"^\s*(\d+)\s*([KMGT]?)B?\s*$", re.IGNORECASE)

_MEM_MULTIPLIERS: dict[str, float] = {
    "": 1,
    "K": 1 / 1024,    # KB → MB
    "M": 1,
    "G": 1024,
    "T": 1024 * 1024,
}


def parse_memory(mem_str: str) -> int:
    """Parse a memory string like ``"4G"`` into megabytes (int).

    Accepted suffixes: ``K``, ``M``, ``G``, ``T`` (case-insensitive).
    A bare number is treated as megabytes.
    """
    match = _MEM_RE.match(mem_str)
    if not match:
        raise ValueError(f"Invalid memory format: {mem_str!r}")
    value = int(match.group(1))
    suffix = match.group(2).upper()
    return max(1, int(value * _MEM_MULTIPLIERS.get(suffix, 1)))


_JOB_NAME_RE = re.compile(r"^[a-zA-Z0-9][a-zA-Z0-9_.@:+/-]*$")


def validate_job_name(name: str) -> str:
    """Return a sanitised job name, or raise on invalid input."""
    name = name.strip()
    if not name:
        raise ValueError("Job name must not be empty")
    if len(name) > 200:
        raise ValueError("Job name too long (max 200 chars)")
    if not _JOB_NAME_RE.match(name):
        raise ValueError(
            "Job name contains invalid characters "
            "(use letters, digits, dots, underscores, @, colons, +, /, hyphens)"
        )
    return name
