"""Rich formatting helpers for SlurmTerm."""

from __future__ import annotations

from rich.markup import escape as escape_markup
from rich.style import Style
from rich.text import Text

# Mapping from Slurm job state → Rich colour name.
_STATE_COLORS: dict[str, str] = {
    "RUNNING": "green",
    "COMPLETING": "green",
    "COMPLETED": "bright_green",
    "PENDING": "yellow",
    "SUSPENDED": "bright_yellow",
    "FAILED": "red",
    "TIMEOUT": "red",
    "CANCELLED": "bright_red",
    "NODE_FAIL": "red",
    "PREEMPTED": "magenta",
    "OUT_OF_MEMORY": "red",
}


def state_color(state: str) -> str:
    """Return a Rich colour name for a Slurm job *state*."""
    return _STATE_COLORS.get(state.upper(), "white")


def state_style(state: str) -> Style:
    """Return a :class:`rich.style.Style` for a Slurm job *state*."""
    return Style(color=state_color(state))


def styled_state(state: str) -> Text:
    """Return a :class:`rich.text.Text` with the state coloured."""
    return Text(state, style=state_style(state))
