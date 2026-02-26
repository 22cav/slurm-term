"""Queue Monitor tab — real-time job table with auto-polling."""

from __future__ import annotations

import asyncio

from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Vertical
from textual.message import Message
from textual.widgets import DataTable, Static

from slurm_term.slurm_api import JobInfo, SlurmController
from slurm_term.utils.formatting import escape_markup, styled_state

_COLUMNS = ("JobID", "Name", "Partition", "State", "Time", "Nodes", "Reason")


class MonitorTab(Vertical):
    """Job queue monitor with auto-refreshing DataTable."""

    class InspectJobRequested(Message):
        """Posted when the user wants to inspect a job."""

        def __init__(self, job_id: str) -> None:
            super().__init__()
            self.job_id = job_id

    BINDINGS = [
        Binding("k", "cancel_job", "Kill Job", show=True),
        Binding("h", "hold_job", "Hold Job", show=True),
        Binding("u", "release_job", "Release Job", show=True),
        Binding("r", "refresh", "Refresh", show=True),
        Binding("i", "inspect_job", "Inspect", show=True),
    ]

    DEFAULT_CSS = """
    MonitorTab {
        height: 1fr;
    }
    #queue-table {
        height: 1fr;
        scrollbar-size: 1 1;
    }
    """

    def __init__(
        self,
        slurm: SlurmController | None = None,
        poll_interval: float = 3.0,
        **kwargs,
    ) -> None:
        super().__init__(**kwargs)
        self.slurm = slurm or SlurmController()
        self.poll_interval = poll_interval
        self._jobs: dict[str, JobInfo] = {}

    def compose(self) -> ComposeResult:
        yield DataTable(id="queue-table", cursor_type="row")
        yield Static("Loading queue…", id="monitor-status", classes="status-bar")

    def on_mount(self) -> None:
        table = self.query_one("#queue-table", DataTable)
        for col in _COLUMNS:
            table.add_column(col, key=col.lower())
        self.set_interval(self.poll_interval, self._poll)
        self._poll()

    # ---- data fetching ----------------------------------------------------

    def _poll(self) -> None:
        self.run_worker(self._fetch_queue, exclusive=True, group="queue")

    async def _fetch_queue(self) -> list[JobInfo]:
        loop = asyncio.get_running_loop()
        jobs = await loop.run_in_executor(None, self.slurm.get_queue)
        self._update_table(jobs)
        return jobs

    def _update_table(self, jobs: list[JobInfo]) -> None:
        table = self.query_one("#queue-table", DataTable)
        new_jobs = {j.job_id: j for j in jobs}

        # Remove disappeared rows
        for job_id in set(self._jobs) - set(new_jobs):
            try:
                table.remove_row(job_id)
            except KeyError:
                pass

        # Add or update rows
        for job_id, job in new_jobs.items():
            row = (
                job.job_id, job.name, job.partition,
                styled_state(job.state), job.time_used, job.nodes, job.reason,
            )
            if job_id in self._jobs:
                old = self._jobs[job_id]
                if (old.state, old.time_used, old.reason, old.nodes) != (
                    job.state, job.time_used, job.reason, job.nodes,
                ):
                    for ci, col_key in enumerate(("jobid", "name", "partition", "state", "time", "nodes", "reason")):
                        try:
                            table.update_cell(job_id, col_key, row[ci], update_width=True)
                        except KeyError:
                            pass
            else:
                table.add_row(*row, key=job_id)

        self._jobs = new_jobs
        n = len(new_jobs)
        self._set_status(f"{n} job{'s' if n != 1 else ''} shown")

    # ---- helpers ----------------------------------------------------------

    def _get_selected_job_id(self) -> str | None:
        table = self.query_one("#queue-table", DataTable)
        if table.cursor_row is not None and table.row_count > 0:
            try:
                row_key = table.coordinate_to_cell_key((table.cursor_row, 0)).row_key
                return str(row_key.value)
            except Exception:
                return None
        return None

    def _set_status(self, msg: str) -> None:
        self.query_one("#monitor-status", Static).update(f" {msg}")

    # ---- actions ----------------------------------------------------------

    def action_refresh(self) -> None:
        self._poll()

    def action_cancel_job(self) -> None:
        job_id = self._get_selected_job_id()
        if not job_id:
            return
        from slurm_term.screens.confirm import ConfirmScreen
        self.app.push_screen(
            ConfirmScreen(f"Cancel job [b]{escape_markup(job_id)}[/b]?"),
            callback=lambda ok: self._do_cancel(job_id) if ok else None,
        )

    def _do_cancel(self, job_id: str) -> None:
        self.run_worker(
            lambda: self._run_cancel(job_id), group="job-action", exclusive=True,
        )

    async def _run_cancel(self, job_id: str) -> None:
        loop = asyncio.get_running_loop()
        ok = await loop.run_in_executor(None, self.slurm.cancel_job, job_id)
        if ok:
            self._set_status(f"Cancelled job {job_id}")
        else:
            self._set_status(f"Failed to cancel job {job_id}")
        self._poll()

    def action_hold_job(self) -> None:
        job_id = self._get_selected_job_id()
        if not job_id:
            return
        self.run_worker(
            lambda: self._run_hold(job_id), group="job-action", exclusive=True,
        )

    async def _run_hold(self, job_id: str) -> None:
        loop = asyncio.get_running_loop()
        ok = await loop.run_in_executor(None, self.slurm.hold_job, job_id)
        if ok:
            self._set_status(f"Held job {job_id}")
        else:
            self._set_status(f"Failed to hold job {job_id} (is it PENDING?)")
        self._poll()

    def action_release_job(self) -> None:
        job_id = self._get_selected_job_id()
        if not job_id:
            return
        self.run_worker(
            lambda: self._run_release(job_id), group="job-action", exclusive=True,
        )

    async def _run_release(self, job_id: str) -> None:
        loop = asyncio.get_running_loop()
        ok = await loop.run_in_executor(None, self.slurm.release_job, job_id)
        if ok:
            self._set_status(f"Released job {job_id}")
        else:
            self._set_status(f"Failed to release job {job_id} (is it held?)")
        self._poll()

    def action_inspect_job(self) -> None:
        job_id = self._get_selected_job_id()
        if job_id:
            self.post_message(self.InspectJobRequested(job_id))

    def on_data_table_row_selected(self, event: DataTable.RowSelected) -> None:
        """Handle Enter on a DataTable row — inspect the job."""
        job_id = self._get_selected_job_id()
        if job_id:
            self.post_message(self.InspectJobRequested(job_id))
