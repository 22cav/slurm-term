"""Tests for slurm_term.utils.validators."""

from __future__ import annotations

import pytest

from slurm_term.utils.validators import (
    parse_time,
    format_time,
    parse_memory,
    validate_job_name,
)


# ---------------------------------------------------------------------------
# parse_time
# ---------------------------------------------------------------------------

class TestParseTime:
    def test_seconds_only(self):
        assert parse_time("90") == 90

    def test_mm_ss(self):
        assert parse_time("05:30") == 5 * 60 + 30

    def test_hh_mm_ss(self):
        assert parse_time("02:30:45") == 2 * 3600 + 30 * 60 + 45

    def test_d_hh_mm_ss(self):
        assert parse_time("1-12:00:00") == 1 * 86400 + 12 * 3600

    def test_multi_day(self):
        assert parse_time("3-00:00:00") == 3 * 86400

    def test_whitespace_stripped(self):
        assert parse_time("  01:00:00  ") == 3600

    def test_empty_raises(self):
        with pytest.raises(ValueError, match="Empty"):
            parse_time("")

    def test_blank_raises(self):
        with pytest.raises(ValueError):
            parse_time("   ")

    def test_zero(self):
        assert parse_time("0") == 0
        assert parse_time("00:00:00") == 0


# ---------------------------------------------------------------------------
# format_time
# ---------------------------------------------------------------------------

class TestFormatTime:
    def test_zero(self):
        assert format_time(0) == "00:00:00"

    def test_one_hour(self):
        assert format_time(3600) == "01:00:00"

    def test_complex(self):
        assert format_time(2 * 3600 + 30 * 60 + 15) == "02:30:15"

    def test_with_days(self):
        assert format_time(86400 + 3600) == "1-01:00:00"

    def test_negative_raises(self):
        with pytest.raises(ValueError, match="Negative"):
            format_time(-1)

    def test_roundtrip(self):
        """parse_time(format_time(n)) should equal n."""
        for secs in [0, 1, 60, 3661, 86400, 90061]:
            assert parse_time(format_time(secs)) == secs


# ---------------------------------------------------------------------------
# parse_memory
# ---------------------------------------------------------------------------

class TestParseMemory:
    def test_bare_number(self):
        assert parse_memory("4096") == 4096

    def test_megabytes(self):
        assert parse_memory("512M") == 512

    def test_gigabytes(self):
        assert parse_memory("4G") == 4 * 1024

    def test_terabytes(self):
        assert parse_memory("1T") == 1024 * 1024

    def test_case_insensitive(self):
        assert parse_memory("2g") == 2 * 1024
        assert parse_memory("2GB") == 2 * 1024

    def test_invalid_raises(self):
        with pytest.raises(ValueError, match="Invalid memory"):
            parse_memory("lots")


# ---------------------------------------------------------------------------
# validate_job_name
# ---------------------------------------------------------------------------

class TestValidateJobName:
    def test_valid(self):
        assert validate_job_name("my_training_job") == "my_training_job"

    def test_strips_whitespace(self):
        assert validate_job_name("  job  ") == "job"

    def test_empty_raises(self):
        with pytest.raises(ValueError, match="empty"):
            validate_job_name("")

    def test_too_long_raises(self):
        with pytest.raises(ValueError, match="too long"):
            validate_job_name("x" * 201)


# ---------------------------------------------------------------------------
# parse_memory edge cases
# ---------------------------------------------------------------------------

class TestParseMemoryEdgeCases:
    def test_kilobytes(self):
        # 1024 KB = 1 MB
        assert parse_memory("1024K") == 1

    def test_small_kilobytes_floor_to_1(self):
        # 100 KB < 1 MB, but min is 1
        assert parse_memory("100K") == 1

    def test_terabytes(self):
        assert parse_memory("2T") == 2 * 1024 * 1024

    def test_with_b_suffix(self):
        assert parse_memory("4GB") == 4 * 1024
        assert parse_memory("512MB") == 512

    def test_whitespace(self):
        assert parse_memory("  8G  ") == 8 * 1024
