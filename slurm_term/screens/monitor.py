"""Queue Monitor tab — real-time job table with auto-polling."""

from __future__ import annotations

import asyncio
import time

from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Vertical
from textual.message import Message
from textual.widgets import DataTable, Static, Input

from slurm_term.slurm_api import JobInfo, SlurmController
from slurm_term.utils.formatting import escape_markup, styled_state

from typing import TYPE_CHECKING
if TYPE_CHECKING:
    from slurm_term.config import SlurmTermConfig

_COLUMNS = ("JobID", "Name", "Partition", "State", "Time", "Nodes", "Reason")

_REFRESH_COOLDOWN = 2.0  # minimum seconds between manual refreshes
_PEEK_LINES = 50         # max lines shown in quick-peek


def _read_last_lines(path: str, n: int) -> str:
    """Read the last *n* lines from a text file."""
    try:
        with open(path, "r", errors="replace") as f:
            lines = f.readlines()
            return "".join(lines[-n:]) or "[dim]File is empty.[/dim]"
    except (FileNotFoundError, PermissionError, OSError) as e:
        return f"[red]Cannot read file: {e}[/red]"


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
        Binding("o", "peek_output", "Peek Output", show=True),
        Binding("space", "toggle_select", "Select", show=False),
        Binding("ctrl+a", "select_all", "Select All", show=False),
        Binding("slash", "focus_search", "Search", show=True),
    ]

    DEFAULT_CSS = """
    MonitorTab {
        height: 1fr;
    }
    #queue-search {
        dock: top;
        display: none;
        margin: 0 1;
    }
    #queue-search.visible {
        display: block;
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
        config: SlurmTermConfig | None = None,
        **kwargs,
    ) -> None:
        super().__init__(**kwargs)
        self.slurm = slurm or SlurmController()
        self.poll_interval = poll_interval
        self._config = config
        self._jobs: dict[str, JobInfo] = {}
        self._search_query: str = ""
        self._selected: set[str] = set()
        self._last_manual_refresh: float = 0.0

    def compose(self) -> ComposeResult:
        yield Input(placeholder="Filter jobs…", id="queue-search")
        yield DataTable(id="queue-table", cursor_type="row")
        yield Static("Loading queue…", id="monitor-status", classes="status-bar")

    def on_mount(self) -> None:
        table = self.query_one("#queue-table", DataTable)
        for col in _COLUMNS:
            table.add_column(col, key=col.lower())
        self.set_interval(self.poll_interval, self._poll)
        self._poll()

    # ---- search / filter ------------------------------------------------------

    def action_focus_search(self) -> None:
        search = self.query_one("#queue-search", Input)
        search.add_class("visible")
        search.focus()

    def on_input_changed(self, event: Input.Changed) -> None:
        if event.input.id == "queue-search":
            self._search_query = event.value.strip().lower()
            self._rebuild_table()

    def on_input_submitted(self, event: Input.Submitted) -> None:
        if event.input.id == "queue-search":
            self.query_one("#queue-table", DataTable).focus()

    def _on_key(self, event) -> None:
        """Clear selection or search on Escape."""
        if event.key == "escape":
            if self._selected:
                self._selected.clear()
                self._rebuild_table()
                event.prevent_default()
                event.stop()
                return
            search = self.query_one("#queue-search", Input)
            if search.has_focus or self._search_query:
                search.value = ""
                self._search_query = ""
                search.remove_class("visible")
                self.query_one("#queue-table", DataTable).focus()
                self._rebuild_table()
                event.prevent_default()
                event.stop()

    def _filtered_jobs(self) -> dict[str, JobInfo]:
        if not self._search_query:
            return self._jobs
        q = self._search_query
        return {
            jid: j for jid, j in self._jobs.items()
            if q in j.name.lower()
            or q in j.job_id.lower()
            or q in j.state.lower()
            or q in j.partition.lower()
        }

    def _rebuild_table(self) -> None:
        """Rebuild the table from current data applying the search filter."""
        table = self.query_one("#queue-table", DataTable)
        table.clear()
        filtered = self._filtered_jobs()
        for job_id, job in filtered.items():
            mark = "● " if job_id in self._selected else ""
            row = (
                job.job_id, mark + job.name, job.partition,
                styled_state(job.state), job.time_used, job.nodes, job.reason,
            )
            table.add_row(*row, key=job_id)
        self._update_status_line()

    def _update_status_line(self) -> None:
        """Update the status bar with job and selection counts."""
        filtered = self._filtered_jobs()
        n = len(filtered)
        total = len(self._jobs)
        sel = len(self._selected)
        if self._search_query:
            msg = f"{n}/{total} jobs matching '{self._search_query}'"
        else:
            msg = f"{total} job{'s' if total != 1 else ''} shown"
        if sel:
            msg += f" ({sel} selected)"
        self._set_status(msg)

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

        # Detect state changes for notifications (before overwriting self._jobs)
        if self._config and self._jobs:
            for job_id, new_job in new_jobs.items():
                if job_id in self._jobs:
                    old_state = self._jobs[job_id].state.upper()
                    new_state = new_job.state.upper()
                    if old_state != new_state:
                        if new_state == "COMPLETED" and self._config.notify_on_complete:
                            self.app.notify(
                                f"Job {job_id} ({new_job.name}) completed",
                                title="Job Completed",
                                severity="information",
                            )
                        elif new_state in ("FAILED", "TIMEOUT", "NODE_FAIL", "OUT_OF_MEMORY") and self._config.notify_on_fail:
                            self.app.notify(
                                f"Job {job_id} ({new_job.name}) {new_state}",
                                title="Job Failed",
                                severity="error",
                            )

        # Also notify for jobs that disappeared while RUNNING (likely completed)
        if self._config and self._config.notify_on_complete:
            for job_id in set(self._jobs) - set(new_jobs):
                old = self._jobs[job_id]
                if old.state.upper() == "RUNNING":
                    self.app.notify(
                        f"Job {job_id} ({old.name}) finished",
                        title="Job Finished",
                        severity="information",
                    )

        self._jobs = new_jobs
        self._selected &= set(new_jobs)  # prune stale selections

        # If there's an active search filter, do a full rebuild
        if self._search_query:
            self._rebuild_table()
            return

        # Remove disappeared rows
        displayed = set(str(rk.value) for rk in table.rows)
        for row_id in displayed - set(new_jobs):
            try:
                table.remove_row(row_id)
            except KeyError:
                pass

        # Add or update rows
        for job_id, job in new_jobs.items():
            mark = "● " if job_id in self._selected else ""
            row = (
                job.job_id, mark + job.name, job.partition,
                styled_state(job.state), job.time_used, job.nodes, job.reason,
            )
            if job_id in displayed:
                try:
                    for ci, col_key in enumerate(("jobid", "name", "partition", "state", "time", "nodes", "reason")):
                        table.update_cell(job_id, col_key, row[ci], update_width=True)
                except KeyError:
                    table.add_row(*row, key=job_id)
            else:
                table.add_row(*row, key=job_id)

        self._update_status_line()

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
        now = time.monotonic()
        if now - self._last_manual_refresh < _REFRESH_COOLDOWN:
            return
        self._last_manual_refresh = now
        self._poll()

    # ---- selection --------------------------------------------------------

    def _get_action_targets(self) -> list[str]:
        """Return selected job IDs, or the cursor row if nothing selected."""
        if self._selected:
            return list(self._selected)
        jid = self._get_selected_job_id()
        return [jid] if jid else []

    def action_toggle_select(self) -> None:
        jid = self._get_selected_job_id()
        if not jid:
            return
        self._selected.symmetric_difference_update({jid})
        self._rebuild_table()

    def action_select_all(self) -> None:
        visible = set(self._filtered_jobs())
        if self._selected >= visible:
            self._selected.clear()
        else:
            self._selected = visible
        self._rebuild_table()

    # ---- peek output ------------------------------------------------------

    def action_peek_output(self) -> None:
        jid = self._get_selected_job_id()
        if jid:
            self.run_worker(self._do_peek(jid), group="peek", exclusive=True)

    async def _do_peek(self, job_id: str) -> None:
        loop = asyncio.get_running_loop()
        details = await loop.run_in_executor(
            None, self.slurm.get_job_details, job_id,
        )
        if not details:
            self._set_status(f"Could not fetch details for job {job_id}")
            return
        path = details.get("standard_output", "")
        if not path:
            self._set_status("No output file for this job")
            return
        content = await loop.run_in_executor(None, _read_last_lines, path, _PEEK_LINES)
        from slurm_term.screens.peek_screen import PeekScreen
        self.app.push_screen(PeekScreen(
            f"[bold]Output:[/bold] {escape_markup(path)} (last {_PEEK_LINES} lines)",
            content,
        ))

    # ---- bulk actions -----------------------------------------------------

    def _do_bulk(self, targets: list[str], fn, verb: str) -> None:
        self.run_worker(
            self._run_bulk(targets, fn, verb),
            group="job-action", exclusive=True,
        )

    async def _run_bulk(self, job_ids: list[str], fn, verb: str) -> None:
        loop = asyncio.get_running_loop()
        ok_count = 0
        for jid in job_ids:
            ok = await loop.run_in_executor(None, fn, jid)
            if ok:
                ok_count += 1
        self._selected.clear()
        failed = len(job_ids) - ok_count
        if len(job_ids) == 1:
            jid = job_ids[0]
            self._set_status(
                f"{verb} job {jid}" if ok_count
                else f"Failed to {verb.lower()} job {jid}"
            )
        elif failed == 0:
            self._set_status(f"{verb} {ok_count} jobs")
        else:
            self._set_status(f"{verb} {ok_count}, failed {failed}")
        self._poll()

    def action_cancel_job(self) -> None:
        targets = self._get_action_targets()
        if not targets:
            return
        from slurm_term.screens.confirm import ConfirmScreen
        n = len(targets)
        msg = (f"Cancel {n} selected jobs?" if n > 1
               else f"Cancel job [b]{escape_markup(targets[0])}[/b]?")
        self.app.push_screen(
            ConfirmScreen(msg),
            callback=lambda ok, t=targets: (
                self._do_bulk(t, self.slurm.cancel_job, "Cancelled") if ok else None
            ),
        )

    def action_hold_job(self) -> None:
        targets = self._get_action_targets()
        if targets:
            self._do_bulk(targets, self.slurm.hold_job, "Held")

    def action_release_job(self) -> None:
        targets = self._get_action_targets()
        if targets:
            self._do_bulk(targets, self.slurm.release_job, "Released")

    def action_inspect_job(self) -> None:
        job_id = self._get_selected_job_id()
        if job_id:
            self.post_message(self.InspectJobRequested(job_id))

    def on_data_table_row_selected(self, event: DataTable.RowSelected) -> None:
        """Handle Enter on a DataTable row — inspect the job."""
        job_id = self._get_selected_job_id()
        if job_id:
            self.post_message(self.InspectJobRequested(job_id))
