"""Tests for inspector metric parsing helpers."""

from __future__ import annotations

import pytest

from slurm_term.screens.inspector import (
    _parse_rss_to_pct, _parse_cpu_pct, _parse_duration_to_seconds,
)


# ---------------------------------------------------------------------------
# _parse_duration_to_seconds
# ---------------------------------------------------------------------------

class TestParseDurationToSeconds:
    def test_hh_mm_ss(self):
        assert _parse_duration_to_seconds("01:23:45") == pytest.approx(5025.0)

    def test_mm_ss(self):
        assert _parse_duration_to_seconds("05:30") == pytest.approx(330.0)

    def test_dd_hh_mm_ss(self):
        # 2 days + 3 hours + 4 minutes + 5 seconds
        assert _parse_duration_to_seconds("2-03:04:05") == pytest.approx(
            2 * 86400 + 3 * 3600 + 4 * 60 + 5
        )

    def test_fractional_seconds(self):
        assert _parse_duration_to_seconds("00:00:01.500") == pytest.approx(1.5)

    def test_empty_string(self):
        assert _parse_duration_to_seconds("") == 0.0

    def test_invalid_string(self):
        assert _parse_duration_to_seconds("invalid") == 0.0


# ---------------------------------------------------------------------------
# _parse_rss_to_pct
# ---------------------------------------------------------------------------

class TestParseRssToPct:
    def test_megabytes(self):
        # 16000M out of 32000MB total = 50%
        assert _parse_rss_to_pct("16000M", 32000) == pytest.approx(50.0)

    def test_gigabytes(self):
        # 8G = 8192M out of 32000MB ≈ 25.6%
        assert _parse_rss_to_pct("8G", 32000) == pytest.approx(8192 / 32000 * 100)

    def test_kilobytes(self):
        # 1024K = 1M out of 1000MB = 0.1%
        assert _parse_rss_to_pct("1024K", 1000) == pytest.approx(0.1, abs=0.01)

    def test_bytes_assumed(self):
        # 1048576 bytes = 1M out of 1000MB = 0.1%
        assert _parse_rss_to_pct("1048576", 1000) == pytest.approx(0.1, abs=0.01)

    def test_empty_string(self):
        assert _parse_rss_to_pct("", 32000) == 0.0

    def test_zero_total(self):
        assert _parse_rss_to_pct("1000M", 0) == 0.0

    def test_caps_at_100(self):
        assert _parse_rss_to_pct("64000M", 32000) == 100.0

    def test_invalid_string(self):
        assert _parse_rss_to_pct("invalid", 32000) == 0.0


# ---------------------------------------------------------------------------
# _parse_cpu_pct
# ---------------------------------------------------------------------------

class TestParseCpuPct:
    def test_percentage(self):
        assert _parse_cpu_pct("85%") == pytest.approx(85.0)

    def test_caps_at_100(self):
        assert _parse_cpu_pct("150%") == 100.0

    def test_time_format_with_elapsed(self):
        # 30 seconds of CPU in 60 seconds elapsed = 50%
        assert _parse_cpu_pct("00:00:30", elapsed_seconds=60.0) == pytest.approx(50.0)

    def test_time_format_without_elapsed(self):
        # Without elapsed context, can't compute %
        assert _parse_cpu_pct("01:23:45") == 0.0

    def test_time_format_hh_mm_ss(self):
        # 1 hour CPU in 2 hours elapsed = 50%
        assert _parse_cpu_pct("01:00:00", elapsed_seconds=7200.0) == pytest.approx(50.0)

    def test_empty_string(self):
        assert _parse_cpu_pct("") == 0.0

    def test_invalid_pct(self):
        assert _parse_cpu_pct("abc%") == 0.0
