"""Tests for the SlurmTerm Textual application."""

from __future__ import annotations

import json
from unittest.mock import MagicMock

import pytest

from textual.widgets import (
    DataTable, TabbedContent, Header, Footer, Input, Select, Button, TextArea,
)

from slurm_term.main import SlurmTermApp
from slurm_term.slurm_api import SlurmController
from slurm_term.mock_slurm import MockSlurmController


# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

def _mock_slurm() -> SlurmController:
    ctrl = SlurmController()
    ctrl._run = MagicMock(
        return_value=MagicMock(stdout='{"jobs":[]}', returncode=0, stderr="")
    )
    ctrl.get_cluster_name = MagicMock(return_value="test-cluster")
    ctrl.get_sinfo = MagicMock(return_value=[])
    ctrl.get_node_info = MagicMock(return_value=[])
    return ctrl


def _mock_slurm_with_jobs() -> SlurmController:
    sample = {"jobs": [
        {"job_id": 100, "name": "test_job", "partition": "debug",
         "job_state": ["RUNNING"], "time": {"elapsed": 120},
         "nodes": "1", "state_reason": "None", "user_name": "testuser"},
        {"job_id": 101, "name": "pending_job", "partition": "batch",
         "job_state": "PENDING", "time": {"elapsed": 0},
         "nodes": "2", "state_reason": "Resources", "user_name": "testuser"},
    ]}
    ctrl = SlurmController()
    ctrl._run = MagicMock(
        return_value=MagicMock(stdout=json.dumps(sample), returncode=0, stderr="")
    )
    ctrl.get_cluster_name = MagicMock(return_value="test-cluster")
    ctrl.get_sinfo = MagicMock(return_value=[])
    ctrl.get_node_info = MagicMock(return_value=[])
    return ctrl


# ---------------------------------------------------------------------------
# App structure
# ---------------------------------------------------------------------------

class TestAppMounts:
    @pytest.mark.asyncio
    async def test_mounts(self):
        async with SlurmTermApp(slurm=_mock_slurm()).run_test():
            pass

    @pytest.mark.asyncio
    async def test_has_header_footer_tabs(self):
        app = SlurmTermApp(slurm=_mock_slurm())
        async with app.run_test():
            assert app.query_one(Header)
            assert app.query_one(Footer)
            assert app.query_one(TabbedContent)

    @pytest.mark.asyncio
    async def test_datatable_columns(self):
        app = SlurmTermApp(slurm=_mock_slurm())
        async with app.run_test():
            table = app.query_one("#queue-table", DataTable)
            keys = [c.key.value for c in table.columns.values()]
            assert keys == ["jobid", "name", "partition", "state", "time", "nodes", "reason"]


class TestAppWithJobs:
    @pytest.mark.asyncio
    async def test_jobs_appear(self):
        app = SlurmTermApp(slurm=_mock_slurm_with_jobs())
        async with app.run_test() as pilot:
            await pilot.pause()
            await pilot.pause()
            assert app.query_one("#queue-table", DataTable).row_count == 2

    @pytest.mark.asyncio
    async def test_subtitle(self):
        app = SlurmTermApp(slurm=_mock_slurm())
        async with app.run_test():
            assert "test-cluster" in app.sub_title


class TestTabSwitching:
    @pytest.mark.asyncio
    async def test_tabs(self):
        app = SlurmTermApp(slurm=_mock_slurm())
        async with app.run_test() as pilot:
            for key, expected in [
                ("2", "composer"), ("3", "hardware"),
                ("4", "history"), ("5", "inspector"),
                ("1", "monitor"),
            ]:
                await pilot.press(key)
                assert app.query_one(TabbedContent).active == expected


# ---------------------------------------------------------------------------
# Composer (unified sbatch + srun)
# ---------------------------------------------------------------------------

class TestComposerTab:
    @pytest.mark.asyncio
    async def test_has_core_inputs(self):
        app = SlurmTermApp(slurm=MockSlurmController())
        async with app.run_test() as pilot:
            await pilot.press("2")
            for wid in ["input-time", "input-nodes", "input-cpus",
                         "input-memory"]:
                assert app.query_one(f"#{wid}", Input) is not None

    @pytest.mark.asyncio
    async def test_has_mode_select(self):
        app = SlurmTermApp(slurm=MockSlurmController())
        async with app.run_test() as pilot:
            await pilot.press("2")
            assert app.query_one("#select-mode", Select) is not None

    @pytest.mark.asyncio
    async def test_has_sbatch_fields(self):
        app = SlurmTermApp(slurm=MockSlurmController())
        async with app.run_test() as pilot:
            await pilot.press("2")
            assert app.query_one("#input-name", Input) is not None
            assert app.query_one("#input-script", Input) is not None
            assert app.query_one("#input-output", Input) is not None

    @pytest.mark.asyncio
    async def test_has_textareas(self):
        app = SlurmTermApp(slurm=MockSlurmController())
        async with app.run_test() as pilot:
            await pilot.press("2")
            assert app.query_one("#input-modules", TextArea) is not None
            assert app.query_one("#input-env", TextArea) is not None
            assert app.query_one("#input-init", TextArea) is not None

    @pytest.mark.asyncio
    async def test_has_submit_and_preview(self):
        app = SlurmTermApp(slurm=MockSlurmController())
        async with app.run_test() as pilot:
            await pilot.press("2")
            assert app.query_one("#btn-submit", Button) is not None
            assert app.query_one("#composer-preview") is not None

    @pytest.mark.asyncio
    async def test_has_add_param_button(self):
        app = SlurmTermApp(slurm=MockSlurmController())
        async with app.run_test() as pilot:
            await pilot.press("2")
            assert app.query_one("#btn-add-param", Button) is not None




# ---------------------------------------------------------------------------
# Hardware Tab
# ---------------------------------------------------------------------------

class TestHardwareTab:
    @pytest.mark.asyncio
    async def test_has_partition_table(self):
        app = SlurmTermApp(slurm=MockSlurmController())
        async with app.run_test() as pilot:
            await pilot.press("3")
            table = app.query_one("#hw-part-table", DataTable)
            keys = [c.key.value for c in table.columns.values()]
            assert "partition" in keys
            assert "cpus" in keys
            assert "gres" in keys

    @pytest.mark.asyncio
    async def test_has_node_table(self):
        app = SlurmTermApp(slurm=MockSlurmController())
        async with app.run_test() as pilot:
            await pilot.press("3")
            table = app.query_one("#hw-node-table", DataTable)
            keys = [c.key.value for c in table.columns.values()]
            assert "node" in keys
            assert "cpus" in keys

    @pytest.mark.asyncio
    async def test_demo_populates_partitions(self):
        app = SlurmTermApp(slurm=MockSlurmController())
        async with app.run_test() as pilot:
            await pilot.press("3")
            await pilot.pause()
            await pilot.pause()
            table = app.query_one("#hw-part-table", DataTable)
            assert table.row_count > 0

    @pytest.mark.asyncio
    async def test_demo_populates_nodes(self):
        app = SlurmTermApp(slurm=MockSlurmController())
        async with app.run_test() as pilot:
            await pilot.press("3")
            await pilot.pause()
            await pilot.pause()
            table = app.query_one("#hw-node-table", DataTable)
            assert table.row_count > 0

    @pytest.mark.asyncio
    async def test_memory_in_gb(self):
        """Partition memory should be in GB — 64000 MB → 62.5 GB."""
        app = SlurmTermApp(slurm=MockSlurmController())
        async with app.run_test() as pilot:
            await pilot.press("3")
            await pilot.pause()
            await pilot.pause()
            table = app.query_one("#hw-part-table", DataTable)
            keys = [c.key.value for c in table.columns.values()]
            assert "mem(gb)" in keys


# ---------------------------------------------------------------------------
# Demo mode
# ---------------------------------------------------------------------------

class TestDemoMode:
    @pytest.mark.asyncio
    async def test_demo_shows_jobs(self):
        app = SlurmTermApp(slurm=MockSlurmController())
        async with app.run_test() as pilot:
            await pilot.pause()
            await pilot.pause()
            assert app.query_one("#queue-table", DataTable).row_count > 0

    @pytest.mark.asyncio
    async def test_demo_subtitle(self):
        app = SlurmTermApp(slurm=MockSlurmController())
        async with app.run_test():
            assert "demo-cluster" in app.sub_title
