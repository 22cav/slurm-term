"""Tests for slurm_term.slurm_api — SlurmController.

All tests mock subprocess.run so no real Slurm installation is needed.
"""

from __future__ import annotations

import json
import subprocess
from unittest.mock import patch, MagicMock

import pytest

from slurm_term.slurm_api import SlurmController, JobInfo


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

SAMPLE_SQUEUE_JSON = {
    "jobs": [
        {
            "job_id": 12345,
            "name": "train_model",
            "partition": "gpu",
            "job_state": ["RUNNING"],
            "time": {"elapsed": 3661},
            "nodes": "2",
            "state_reason": "None",
            "user_name": "testuser",
            "working_directory": "/home/testuser/project",
            "standard_output": "/home/testuser/project/train-%j.out",
            "standard_error": "/home/testuser/project/train-%j.err",
            "submit_time": {"number": 1700000000},
        },
        {
            "job_id": 12346,
            "name": "preprocess",
            "partition": "batch",
            "job_state": "PENDING",
            "time": {"elapsed": 0},
            "nodes": "1",
            "state_reason": "Resources",
            "user_name": "testuser",
            "working_directory": "/home/testuser/data",
            "standard_output": "/home/testuser/data/pre-%j.out",
            "standard_error": "/home/testuser/data/pre-%j.err",
            "submit_time": {"number": 1700000100},
        },
    ]
}

SAMPLE_SCONTROL_JOB_JSON = {
    "jobs": [
        {
            "job_id": 12345,
            "name": "train_model",
            "partition": "gpu",
            "job_state": "RUNNING",
            "user_name": "testuser",
            "working_directory": "/home/testuser/project",
            "standard_output": "/home/testuser/project/train-12345.out",
            "standard_error": "/home/testuser/project/train-12345.err",
            "nodes": "node[001-002]",
            "submit_time": "2024-01-15T10:00:00",
        }
    ]
}


def _mock_run(stdout: str = "", returncode: int = 0, stderr: str = ""):
    """Create a mock CompletedProcess."""
    cp = MagicMock(spec=subprocess.CompletedProcess)
    cp.stdout = stdout
    cp.returncode = returncode
    cp.stderr = stderr
    return cp


# ---------------------------------------------------------------------------
# Tests — get_queue
# ---------------------------------------------------------------------------

class TestGetQueue:
    """Tests for SlurmController.get_queue."""

    def test_parses_two_jobs(self):
        ctrl = SlurmController()
        with patch.object(
            ctrl, "_run", return_value=_mock_run(json.dumps(SAMPLE_SQUEUE_JSON))
        ):
            jobs = ctrl.get_queue(user="testuser")

        assert len(jobs) == 2
        assert all(isinstance(j, JobInfo) for j in jobs)

    def test_first_job_fields(self):
        ctrl = SlurmController()
        with patch.object(
            ctrl, "_run", return_value=_mock_run(json.dumps(SAMPLE_SQUEUE_JSON))
        ):
            jobs = ctrl.get_queue(user="testuser")

        job = jobs[0]
        assert job.job_id == "12345"
        assert job.name == "train_model"
        assert job.partition == "gpu"
        assert job.state == "RUNNING"
        assert job.time_used == "01:01:01"
        assert job.nodes == "2"

    def test_pending_job_reason(self):
        ctrl = SlurmController()
        with patch.object(
            ctrl, "_run", return_value=_mock_run(json.dumps(SAMPLE_SQUEUE_JSON))
        ):
            jobs = ctrl.get_queue(user="testuser")

        assert jobs[1].state == "PENDING"
        assert jobs[1].reason == "Resources"

    def test_empty_queue(self):
        ctrl = SlurmController()
        empty = {"jobs": []}
        with patch.object(
            ctrl, "_run", return_value=_mock_run(json.dumps(empty))
        ):
            jobs = ctrl.get_queue(user="testuser")
        assert jobs == []

    def test_squeue_failure_returns_empty(self):
        ctrl = SlurmController()
        with patch.object(
            ctrl, "_run", return_value=_mock_run(returncode=1, stderr="error")
        ):
            jobs = ctrl.get_queue(user="testuser")
        assert jobs == []

    def test_invalid_json_returns_empty(self):
        ctrl = SlurmController()
        with patch.object(
            ctrl, "_run", return_value=_mock_run(stdout="not json at all")
        ):
            jobs = ctrl.get_queue(user="testuser")
        assert jobs == []

    def test_state_as_string(self):
        """``job_state`` can be a plain string in some Slurm versions."""
        ctrl = SlurmController()
        with patch.object(
            ctrl, "_run", return_value=_mock_run(json.dumps(SAMPLE_SQUEUE_JSON))
        ):
            jobs = ctrl.get_queue(user="testuser")
        # Second job has state as string
        assert jobs[1].state == "PENDING"


# ---------------------------------------------------------------------------
# Tests — get_partitions
# ---------------------------------------------------------------------------

class TestGetPartitions:
    def test_parses_partitions(self):
        ctrl = SlurmController()
        output = "debug\nbatch*\ngpu\n"
        with patch.object(ctrl, "_run", return_value=_mock_run(output)):
            parts = ctrl.get_partitions()
        assert parts == ["debug", "batch", "gpu"]

    def test_empty_output(self):
        ctrl = SlurmController()
        with patch.object(ctrl, "_run", return_value=_mock_run("")):
            parts = ctrl.get_partitions()
        assert parts == []

    def test_failure_returns_empty(self):
        ctrl = SlurmController()
        with patch.object(ctrl, "_run", return_value=_mock_run(returncode=1)):
            parts = ctrl.get_partitions()
        assert parts == []


# ---------------------------------------------------------------------------
# Tests — cancel_job
# ---------------------------------------------------------------------------

class TestCancelJob:
    def test_success(self):
        ctrl = SlurmController()
        with patch.object(ctrl, "_run", return_value=_mock_run(returncode=0)):
            assert ctrl.cancel_job("12345") is True

    def test_failure(self):
        ctrl = SlurmController()
        with patch.object(ctrl, "_run", return_value=_mock_run(returncode=1)):
            assert ctrl.cancel_job("12345") is False


# ---------------------------------------------------------------------------
# Tests — hold / release
# ---------------------------------------------------------------------------

class TestHoldRelease:
    def test_hold_success(self):
        ctrl = SlurmController()
        with patch.object(ctrl, "_run", return_value=_mock_run(returncode=0)):
            assert ctrl.hold_job("12345") is True

    def test_release_success(self):
        ctrl = SlurmController()
        with patch.object(ctrl, "_run", return_value=_mock_run(returncode=0)):
            assert ctrl.release_job("12345") is True


# ---------------------------------------------------------------------------
# Tests — get_job_details
# ---------------------------------------------------------------------------

class TestGetJobDetails:
    def test_returns_job_dict(self):
        ctrl = SlurmController()
        with patch.object(
            ctrl,
            "_run",
            return_value=_mock_run(json.dumps(SAMPLE_SCONTROL_JOB_JSON)),
        ):
            details = ctrl.get_job_details("12345")
        assert details["job_id"] == 12345
        assert details["name"] == "train_model"

    def test_not_found_returns_empty(self):
        ctrl = SlurmController()
        with patch.object(ctrl, "_run", return_value=_mock_run(returncode=1)):
            details = ctrl.get_job_details("99999")
        assert details == {}


# ---------------------------------------------------------------------------
# Tests — submit_job
# ---------------------------------------------------------------------------

class TestSubmitJob:
    def test_success(self):
        ctrl = SlurmController()
        with patch.object(
            ctrl,
            "_run",
            return_value=_mock_run(stdout="Submitted batch job 54321"),
        ):
            job_id = ctrl.submit_job("/path/to/script.sh")
        assert job_id == "54321"

    def test_with_params(self):
        ctrl = SlurmController()
        with patch.object(
            ctrl,
            "_run",
            return_value=_mock_run(stdout="Submitted batch job 54322"),
        ) as mock:
            ctrl.submit_job(
                "/path/to/script.sh",
                {"partition": "gpu", "time": "01:00:00"},
            )
        # Verify the command included the parameters
        call_args = mock.call_args[0][0]
        assert "--partition=gpu" in call_args
        assert "--time=01:00:00" in call_args

    def test_failure_raises(self):
        ctrl = SlurmController()
        with patch.object(
            ctrl,
            "_run",
            return_value=_mock_run(returncode=1, stderr="sbatch: error"),
        ):
            with pytest.raises(RuntimeError, match="sbatch failed"):
                ctrl.submit_job("/path/to/script.sh")


# ---------------------------------------------------------------------------
# Tests — get_cluster_name
# ---------------------------------------------------------------------------

class TestGetClusterName:
    def test_parses_cluster_name(self):
        ctrl = SlurmController()
        output = (
            "Configuration data as of ...\n"
            "ClusterName              = mycluster\n"
            "SomeOtherKey             = value\n"
        )
        with patch.object(ctrl, "_run", return_value=_mock_run(output)):
            assert ctrl.get_cluster_name() == "mycluster"

    def test_failure_returns_unknown(self):
        ctrl = SlurmController()
        with patch.object(ctrl, "_run", return_value=_mock_run(returncode=1)):
            assert ctrl.get_cluster_name() == "unknown"


# ---------------------------------------------------------------------------
# Tests — get_sacct
# ---------------------------------------------------------------------------

class TestGetSacct:
    def test_parses_rows(self):
        ctrl = SlurmController()
        output = (
            "12345|train|gpu|COMPLETED|01:30:00|02:45:00|4096M|0:0\n"
            "12345.batch|train|gpu|COMPLETED|01:30:00|02:45:00|4096M|0:0\n"
            "12346|preprocess|batch|FAILED|00:05:00|00:04:30|512M|1:0\n"
        )
        with patch.object(ctrl, "_run", return_value=_mock_run(output)):
            rows = ctrl.get_sacct(user="testuser")
        # Sub-steps (12345.batch) should be filtered out
        assert len(rows) == 2
        assert rows[0]["job_id"] == "12345"
        assert rows[0]["state"] == "COMPLETED"
        assert rows[1]["job_id"] == "12346"
        assert rows[1]["state"] == "FAILED"

    def test_failure_returns_empty(self):
        ctrl = SlurmController()
        with patch.object(ctrl, "_run", return_value=_mock_run(returncode=1)):
            assert ctrl.get_sacct(user="testuser") == []


# ---------------------------------------------------------------------------
# Tests — get_sstat
# ---------------------------------------------------------------------------

class TestGetSstat:
    def test_parses_metrics(self):
        ctrl = SlurmController()
        output = "01:23:45|2048M|4096M\n"
        with patch.object(ctrl, "_run", return_value=_mock_run(output)):
            stats = ctrl.get_sstat("12345")
        assert stats["avg_cpu"] == "01:23:45"
        assert stats["max_rss"] == "2048M"
        assert stats["max_vmsize"] == "4096M"

    def test_failure_returns_empty(self):
        ctrl = SlurmController()
        with patch.object(ctrl, "_run", return_value=_mock_run(returncode=1)):
            assert ctrl.get_sstat("12345") == {}

    def test_invalid_job_id_raises(self):
        ctrl = SlurmController()
        with pytest.raises(ValueError, match="Invalid job ID"):
            ctrl.get_sstat("not-a-number")

    def test_tries_batch_suffix_fallback(self):
        """When bare job ID returns empty output, .batch suffix is tried."""
        ctrl = SlurmController()
        empty_result = _mock_run(stdout="\n")
        batch_result = _mock_run(stdout="00:05:30|1024M|2048M\n")

        call_count = 0
        def _side_effect(cmd, **kwargs):
            nonlocal call_count
            call_count += 1
            if call_count == 1:
                return empty_result
            return batch_result

        with patch.object(ctrl, "_run", side_effect=_side_effect):
            stats = ctrl.get_sstat("12345")

        assert call_count == 2
        assert stats["avg_cpu"] == "00:05:30"
        assert stats["max_rss"] == "1024M"

    def test_bare_id_succeeds_no_batch_suffix_attempted(self):
        """When bare job ID returns data, .batch suffix is not attempted."""
        ctrl = SlurmController()
        call_count = 0
        def _side_effect(cmd, **kwargs):
            nonlocal call_count
            call_count += 1
            return _mock_run(stdout="00:10:00|512M|1024M\n")

        with patch.object(ctrl, "_run", side_effect=_side_effect):
            stats = ctrl.get_sstat("12345")

        assert call_count == 1
        assert stats["avg_cpu"] == "00:10:00"


# ---------------------------------------------------------------------------
# Tests — get_sinfo
# ---------------------------------------------------------------------------

class TestGetSinfo:
    def test_parses_sinfo(self):
        ctrl = SlurmController()
        output = "debug*|up|00:30:00|4|idle|node[001-004]|16|64000|(null)\n"
        with patch.object(ctrl, "_run", return_value=_mock_run(output)):
            rows = ctrl.get_sinfo()
        assert len(rows) == 1
        assert rows[0]["partition"] == "debug"
        assert rows[0]["avail"] == "up"
        assert rows[0]["nodes"] == "4"

    def test_failure_returns_empty(self):
        ctrl = SlurmController()
        with patch.object(ctrl, "_run", return_value=_mock_run(returncode=1)):
            assert ctrl.get_sinfo() == []


# ---------------------------------------------------------------------------
# Tests — get_node_info
# ---------------------------------------------------------------------------

class TestGetNodeInfo:
    def test_parses_nodes(self):
        ctrl = SlurmController()
        output = (
            "NodeName=node001 CPUTot=16 RealMemory=64000 State=IDLE\n"
            "\n"
            "NodeName=node002 CPUTot=32 RealMemory=128000 State=MIXED\n"
        )
        with patch.object(ctrl, "_run", return_value=_mock_run(output)):
            nodes = ctrl.get_node_info()
        assert len(nodes) == 2
        assert nodes[0]["NodeName"] == "node001"
        assert nodes[1]["State"] == "MIXED"

    def test_failure_returns_empty(self):
        ctrl = SlurmController()
        with patch.object(ctrl, "_run", return_value=_mock_run(returncode=1)):
            assert ctrl.get_node_info() == []


# ---------------------------------------------------------------------------
# Tests — safety validations
# ---------------------------------------------------------------------------

class TestSafetyValidations:
    def test_invalid_job_id_cancel(self):
        ctrl = SlurmController()
        with pytest.raises(ValueError, match="Invalid job ID"):
            ctrl.cancel_job("rm -rf /")

    def test_invalid_job_id_hold(self):
        ctrl = SlurmController()
        with pytest.raises(ValueError, match="Invalid job ID"):
            ctrl.hold_job("$(evil)")

    def test_unsafe_param_key_submit(self):
        ctrl = SlurmController()
        with pytest.raises(ValueError, match="Unsafe parameter key"):
            ctrl.submit_job("/tmp/script.sh", {"--injected": "val"})

    def test_array_job_id_accepted(self):
        ctrl = SlurmController()
        with patch.object(ctrl, "_run", return_value=_mock_run(returncode=0)):
            assert ctrl.cancel_job("12345_1") is True
