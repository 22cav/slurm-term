"""SlurmTerm configuration — loads from ~/.config/slurmterm/config.toml."""

from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

try:
    import tomllib  # type: ignore[import-not-found]
except ModuleNotFoundError:
    try:
        import tomli as tomllib  # type: ignore[no-redef,import-not-found]
    except ModuleNotFoundError:
        tomllib = None  # type: ignore[assignment]


CONFIG_PATH = Path(os.environ.get(
    "SLURMTERM_CONFIG",
    Path.home() / ".config" / "slurmterm" / "config.toml",
))


@dataclass
class SlurmTermConfig:
    """Application configuration with sensible defaults."""

    # Polling intervals (seconds)
    monitor_poll_interval: float = 3.0
    inspector_poll_interval: float = 3.0
    hardware_poll_interval: float = 30.0
    history_poll_interval: float = 60.0

    # Subprocess timeout (seconds)
    subprocess_timeout: float = 30.0

    # History
    history_window: str = "now-7days"

    # GPU monitoring
    gpu_monitor_enabled: bool = False
    gpu_monitor_command: str = "nvidia-smi"

    # Notifications
    notify_on_complete: bool = False
    notify_on_fail: bool = False


def load_config(path: Path | None = None) -> SlurmTermConfig:
    """Load config from TOML file, falling back to defaults on any error."""
    cfg = SlurmTermConfig()
    config_path = path or CONFIG_PATH
    if not config_path.is_file():
        return cfg
    if tomllib is None:
        return cfg

    try:
        with open(config_path, "rb") as f:
            data = tomllib.load(f)
    except Exception:
        return cfg

    poll = data.get("poll", {})
    for key, attr in [
        ("monitor", "monitor_poll_interval"),
        ("inspector", "inspector_poll_interval"),
        ("hardware", "hardware_poll_interval"),
        ("history", "history_poll_interval"),
    ]:
        if key in poll:
            try:
                setattr(cfg, attr, float(poll[key]))
            except (ValueError, TypeError):
                pass

    general = data.get("general", {})
    if "subprocess_timeout" in general:
        try:
            cfg.subprocess_timeout = float(general["subprocess_timeout"])
        except (ValueError, TypeError):
            pass
    if "history_window" in general:
        cfg.history_window = str(general["history_window"])

    gpu = data.get("gpu", {})
    if "enabled" in gpu:
        cfg.gpu_monitor_enabled = bool(gpu["enabled"])
    if "command" in gpu:
        cfg.gpu_monitor_command = str(gpu["command"])

    notifications = data.get("notifications", {})
    if "on_complete" in notifications:
        cfg.notify_on_complete = bool(notifications["on_complete"])
    if "on_fail" in notifications:
        cfg.notify_on_fail = bool(notifications["on_fail"])

    return cfg
