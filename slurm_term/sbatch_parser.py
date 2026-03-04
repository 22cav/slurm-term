"""Parse .sbatch files into Composer form state dicts.

Converts ``#SBATCH`` directives, ``module load`` lines, ``export`` lines,
and remaining shell commands into a dict compatible with
:meth:`ComposerTab.set_form_state`.
"""

from __future__ import annotations

import re
from pathlib import Path
from typing import Any


# Maps #SBATCH keys → form state keys
_DIRECTIVE_MAP: dict[str, str] = {
    "job-name": "name",
    "J":        "name",
    "partition": "partition",
    "p":        "partition",
    "time":     "time",
    "t":        "time",
    "nodes":    "nodes",
    "N":        "nodes",
    "ntasks-per-node": "ntasks",
    "cpus-per-task":   "cpus",
    "c":        "cpus",
    "mem":      "memory",
    "output":   "output",
    "o":        "output",
    "error":    "error",
    "e":        "error",
}

# These directives map to the GPU field but need value extraction
_GPU_DIRECTIVES = {"gres", "gpus", "gpus-per-node", "G"}

# Regex for #SBATCH lines: --key=value  or  --key value  or  -X value
_SBATCH_LONG_RE = re.compile(
    r"^\s*#SBATCH\s+--([a-zA-Z][a-zA-Z0-9_-]*)(?:=|\s+)(.+)?$"
)
_SBATCH_SHORT_RE = re.compile(
    r"^\s*#SBATCH\s+-([a-zA-Z])(?:\s+(.+))?$"
)

# Regex for module load lines
_MODULE_LOAD_RE = re.compile(r"^\s*module\s+load\s+(.+)$", re.IGNORECASE)

# Regex for export lines
_EXPORT_RE = re.compile(r"^\s*export\s+([A-Za-z_][A-Za-z0-9_]*=.+)$")


def parse_sbatch_file(path: str | Path) -> dict[str, Any]:
    """Parse an sbatch script file and return a Composer form state dict.

    Parameters
    ----------
    path : str or Path
        Absolute or relative path to the ``.sbatch`` file.

    Returns
    -------
    dict[str, Any]
        A dict with keys matching ``ComposerTab.set_form_state()`` format:
        ``mode``, ``name``, ``partition``, ``time``, ``nodes``, ``ntasks``,
        ``cpus``, ``memory``, ``gpus``, ``output``, ``error``, ``modules``,
        ``env``, ``init``.  An additional ``extra_directives`` key holds
        any unrecognised ``#SBATCH`` flags as a ``dict[str, str]``.

    Raises
    ------
    FileNotFoundError
        If *path* does not exist.
    ValueError
        If *path* is not a file or cannot be read.
    """
    p = Path(path)
    if not p.is_file():
        raise FileNotFoundError(f"File not found: {p}")

    try:
        text = p.read_text(encoding="utf-8", errors="replace")
    except OSError as e:
        raise ValueError(f"Cannot read file: {e}") from e
    return parse_sbatch_text(text)


def parse_sbatch_text(text: str) -> dict[str, Any]:
    """Parse raw sbatch script text into a Composer form state dict.

    This is the core parser; :func:`parse_sbatch_file` reads the file and
    delegates here.
    """
    state: dict[str, str] = {
        "mode": "sbatch",
        "name": "",
        "partition": "",
        "time": "",
        "nodes": "",
        "ntasks": "",
        "cpus": "",
        "memory": "",
        "gpus": "",
        "output": "",
        "error": "",
        "script": "",
    }
    extra_directives: dict[str, str] = {}
    modules: list[str] = []
    env_vars: list[str] = []
    init_cmds: list[str] = []

    # Track whether we've finished the directive block
    past_directives = False

    for line in text.splitlines():
        stripped = line.strip()

        # Skip shebang
        if stripped.startswith("#!"):
            continue

        # Skip pure comments (but not #SBATCH)
        if stripped.startswith("#") and not stripped.startswith("#SBATCH"):
            continue

        # Parse #SBATCH directives
        if stripped.startswith("#SBATCH"):
            m = _SBATCH_LONG_RE.match(stripped)
            if m:
                key, value = m.group(1), (m.group(2) or "").strip()
                _apply_directive(key, value, state, extra_directives)
                continue
            m = _SBATCH_SHORT_RE.match(stripped)
            if m:
                key, value = m.group(1), (m.group(2) or "").strip()
                _apply_directive(key, value, state, extra_directives)
                continue
            # Malformed #SBATCH line — skip
            continue

        # Empty lines before any commands
        if not stripped and not past_directives:
            continue

        past_directives = True

        # Skip empty lines in command section
        if not stripped:
            # Preserve blank lines in init commands for readability
            if init_cmds:
                init_cmds.append("")
            continue

        # Module loads
        m = _MODULE_LOAD_RE.match(stripped)
        if m:
            modules.append(m.group(1).strip())
            continue

        # Export lines
        m = _EXPORT_RE.match(stripped)
        if m:
            env_vars.append(m.group(1).strip())
            continue

        # Everything else is an init command
        init_cmds.append(line.rstrip())

    # Strip trailing blank lines from init_cmds
    while init_cmds and not init_cmds[-1].strip():
        init_cmds.pop()

    state["modules"] = "\n".join(modules)
    state["env"] = "\n".join(env_vars)
    state["init"] = "\n".join(init_cmds)

    result: dict[str, Any] = dict(state)
    if extra_directives:
        result["extra_directives"] = extra_directives
    return result


def _apply_directive(
    key: str,
    value: str,
    state: dict[str, str],
    extras: dict[str, str],
) -> None:
    """Apply a single #SBATCH directive to the state or extras dict."""
    # Check direct mapping first
    if key in _DIRECTIVE_MAP:
        form_key = _DIRECTIVE_MAP[key]
        state[form_key] = value
        return

    # GPU handling: extract count from gres=gpu:N or --gpus=N
    if key in _GPU_DIRECTIVES:
        gpu_val = value
        # Strip "gpu:" prefix if present (e.g. gres=gpu:a100:2 → a100:2)
        if gpu_val.lower().startswith("gpu:"):
            gpu_val = gpu_val[4:]
        state["gpus"] = gpu_val
        return

    # Unrecognised → store as extra directive
    extras[key] = value
