"""Tests for slurm_term.utils.formatting."""

from __future__ import annotations

from rich.style import Style

from slurm_term.utils.formatting import state_color, state_style, styled_state


class TestStateColor:
    def test_running(self):
        assert state_color("RUNNING") == "green"

    def test_pending(self):
        assert state_color("PENDING") == "yellow"

    def test_failed(self):
        assert state_color("FAILED") == "red"

    def test_timeout(self):
        assert state_color("TIMEOUT") == "red"

    def test_completed(self):
        assert state_color("COMPLETED") == "bright_green"

    def test_cancelled(self):
        assert state_color("CANCELLED") == "bright_red"

    def test_unknown_defaults_to_white(self):
        assert state_color("SOME_WEIRD_STATE") == "white"

    def test_case_insensitive(self):
        assert state_color("running") == "green"
        assert state_color("Pending") == "yellow"


class TestStateStyle:
    def test_returns_style_object(self):
        s = state_style("RUNNING")
        assert isinstance(s, Style)
        assert s.color is not None
        assert s.color.name == "green"


class TestStyledState:
    def test_returns_text(self):
        t = styled_state("PENDING")
        assert str(t) == "PENDING"
