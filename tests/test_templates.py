"""Tests for slurm_term.screens.templates — save/load/delete/validate."""

from __future__ import annotations

import json
import os
import tempfile

import pytest

from slurm_term.screens.templates import (
    _sanitize_template_name,
    list_templates,
    save_template,
    load_template,
    delete_template,
    TEMPLATES_DIR,
)


# ---------------------------------------------------------------------------
# _sanitize_template_name
# ---------------------------------------------------------------------------

class TestSanitizeTemplateName:
    def test_valid_name(self):
        assert _sanitize_template_name("my_template") == "my_template"

    def test_strips_whitespace(self):
        assert _sanitize_template_name("  test  ") == "test"

    def test_alphanumeric_with_hyphens(self):
        assert _sanitize_template_name("gpu-job-v2") == "gpu-job-v2"

    def test_with_spaces(self):
        assert _sanitize_template_name("My Template") == "My Template"

    def test_empty_raises(self):
        with pytest.raises(ValueError, match="empty"):
            _sanitize_template_name("")

    def test_special_chars_raise(self):
        with pytest.raises(ValueError, match="Invalid"):
            _sanitize_template_name("bad!!name")

    def test_path_traversal_raises(self):
        with pytest.raises(ValueError, match="Invalid"):
            _sanitize_template_name("../../../etc/passwd")

    def test_too_long_raises(self):
        with pytest.raises(ValueError, match="too long"):
            _sanitize_template_name("x" * 101)

    def test_starts_with_space_raises(self):
        # After strip, " " becomes empty
        with pytest.raises(ValueError, match="empty"):
            _sanitize_template_name("   ")


# ---------------------------------------------------------------------------
# save / load / delete (using a temp directory)
# ---------------------------------------------------------------------------

class TestTemplateIO:
    @pytest.fixture(autouse=True)
    def _use_tmpdir(self, monkeypatch, tmp_path):
        """Redirect TEMPLATES_DIR to a temp directory for tests."""
        import slurm_term.screens.templates as mod
        monkeypatch.setattr(mod, "TEMPLATES_DIR", tmp_path)
        self.tmp = tmp_path

    def test_save_and_load(self):
        data = {"mode": "sbatch", "time": "01:00:00"}
        save_template("test_tmpl", data)
        loaded = load_template("test_tmpl")
        assert loaded == data

    def test_list_templates(self):
        save_template("beta", {"x": 1})
        save_template("alpha", {"y": 2})
        names = list_templates()
        assert names == ["alpha", "beta"]  # sorted

    def test_load_nonexistent_returns_none(self):
        assert load_template("nope") is None

    def test_delete(self):
        save_template("doomed", {"z": 0})
        assert delete_template("doomed") is True
        assert load_template("doomed") is None

    def test_delete_nonexistent_returns_false(self):
        assert delete_template("nope") is False

    def test_load_corrupted_json_returns_none(self):
        bad_file = self.tmp / "corrupt.json"
        bad_file.write_text("not valid json {{{")
        assert load_template("corrupt") is None

    def test_overwrite_existing(self):
        save_template("over", {"v": 1})
        save_template("over", {"v": 2})
        assert load_template("over") == {"v": 2}


