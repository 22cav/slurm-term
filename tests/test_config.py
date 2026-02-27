"""Tests for slurm_term.config — configuration loading."""

from __future__ import annotations

import tempfile
from pathlib import Path

from slurm_term.config import SlurmTermConfig, load_config


class TestDefaults:
    def test_default_values(self):
        cfg = SlurmTermConfig()
        assert cfg.monitor_poll_interval == 3.0
        assert cfg.subprocess_timeout == 30.0
        assert cfg.history_window == "now-7days"
        assert cfg.gpu_monitor_enabled is False
        assert cfg.notify_on_complete is False

    def test_load_missing_file_returns_defaults(self):
        cfg = load_config(Path("/tmp/nonexistent_config.toml"))
        assert cfg.monitor_poll_interval == 3.0


class TestLoadConfig:
    def test_load_custom_values(self):
        with tempfile.NamedTemporaryFile(mode="wb", suffix=".toml", delete=False) as f:
            f.write(b"""
[poll]
monitor = 5.0
hardware = 60.0

[general]
subprocess_timeout = 15.0
history_window = "now-14days"

[gpu]
enabled = true

[notifications]
on_fail = true
""")
            f.flush()
            cfg = load_config(Path(f.name))

        assert cfg.monitor_poll_interval == 5.0
        assert cfg.hardware_poll_interval == 60.0
        assert cfg.inspector_poll_interval == 3.0  # unchanged default
        assert cfg.subprocess_timeout == 15.0
        assert cfg.history_window == "now-14days"
        assert cfg.gpu_monitor_enabled is True
        assert cfg.notify_on_fail is True
        assert cfg.notify_on_complete is False  # unchanged default

    def test_invalid_toml_returns_defaults(self):
        with tempfile.NamedTemporaryFile(mode="wb", suffix=".toml", delete=False) as f:
            f.write(b"this is not valid toml [[[")
            f.flush()
            cfg = load_config(Path(f.name))
        assert cfg.monitor_poll_interval == 3.0
