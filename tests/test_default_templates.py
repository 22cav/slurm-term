"""Tests for slurm_term.default_templates — built-in template seeding."""

from __future__ import annotations

import tempfile
from pathlib import Path
from unittest.mock import patch

from slurm_term.default_templates import ensure_default_templates, DEFAULT_TEMPLATES
from slurm_term.screens.templates import list_templates, load_template, save_template


class TestEnsureDefaultTemplates:
    def test_creates_five_templates_in_empty_dir(self, tmp_path):
        with patch("slurm_term.default_templates.TEMPLATES_DIR", tmp_path), \
             patch("slurm_term.screens.templates.TEMPLATES_DIR", tmp_path):
            ensure_default_templates()
            templates = list_templates()
        assert len(templates) == 5
        assert "Quick CPU Job" in templates
        assert "Interactive Session" in templates

    def test_does_not_overwrite_existing_templates(self, tmp_path):
        with patch("slurm_term.default_templates.TEMPLATES_DIR", tmp_path), \
             patch("slurm_term.screens.templates.TEMPLATES_DIR", tmp_path):
            save_template("My Custom Job", {"mode": "sbatch", "time": "01:00:00"})
            ensure_default_templates()
            templates = list_templates()
        # Should only have the custom template, not the defaults
        assert "My Custom Job" in templates
        assert len(templates) == 1

    def test_default_templates_have_valid_structure(self):
        required_keys = {"mode", "name", "partition", "time", "nodes", "ntasks",
                         "cpus", "memory", "gpus", "script", "output", "error",
                         "modules", "env", "init"}
        for name, data in DEFAULT_TEMPLATES.items():
            assert set(data.keys()) == required_keys, f"Template '{name}' has wrong keys"
            for key, value in data.items():
                assert isinstance(value, str), f"Template '{name}' key '{key}' is not str"

    def test_templates_are_loadable(self, tmp_path):
        with patch("slurm_term.default_templates.TEMPLATES_DIR", tmp_path), \
             patch("slurm_term.screens.templates.TEMPLATES_DIR", tmp_path):
            ensure_default_templates()
            for name in DEFAULT_TEMPLATES:
                data = load_template(name)
                assert data is not None, f"Template '{name}' failed to load"
                assert data["mode"] in ("sbatch", "srun")
