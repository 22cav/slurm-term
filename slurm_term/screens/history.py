"""History / Accounting tab — completed jobs from sacct."""

from __future__ import annotations

import asyncio
import time

from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Vertical
from textual.message import Message
from textual.widgets import DataTable, Static, Select

from slurm_term.slurm_api import SlurmController
from slurm_term.utils.formatting import escape_markup, state_color

_COLS = ("JobID", "Name", "Partition", "State", "Elapsed", "TotalCPU", "MaxRSS", "Exit")

_TIME_WINDOWS = [
    ("1 day", "now-1days"),
    ("3 days", "now-3days"),
    ("7 days", "now-7days"),
    ("14 days", "now-14days"),
    ("30 days", "now-30days"),
]


class HistoryTab(Vertical):
    """Completed job history from sacct."""

    class InspectJobRequested(Message):
        """Posted when the user wants to inspect a completed job."""

        def __init__(self, job_id: str) -> None:
            super().__init__()
            self.job_id = job_id

    class ResubmitRequested(Message):
        """Posted when the user wants to resubmit a completed job."""

        def __init__(self, job_id: str) -> None:
            super().__init__()
            self.job_id = job_id

    BINDINGS = [
        Binding("r", "refresh", "Refresh", show=True),
        Binding("i", "inspect_job", "Inspect", show=True),
        Binding("s", "resubmit", "Resubmit", show=True),
    ]

    DEFAULT_CSS = """
    HistoryTab {
        height: 1fr;
    }
    #history-window-select {
        dock: top;
        width: 30;
        margin: 0 1;
    }
    #history-table {
        height: 1fr;
        scrollbar-size: 1 1;
    }
    """

    def __init__(
        self,
        slurm: SlurmController | None = None,
        poll_interval: float = 60.0,
        history_window: str = "now-7days",
        **kwargs,
    ) -> None:
        super().__init__(**kwargs)
        self.slurm = slurm or SlurmController()
        self.poll_interval = poll_interval
        self._history_window = history_window
        self._last_manual_refresh: float = 0.0

    def compose(self) -> ComposeResult:
        options = [(label, value) for label, value in _TIME_WINDOWS]
        yield Select(
            options,
            value=self._history_window,
            id="history-window-select",
            allow_blank=False,
        )
        yield DataTable(id="history-table", cursor_type="row")
        yield Static("Loading job history…", id="history-status", classes="status-bar")

    def on_mount(self) -> None:
        table = self.query_one("#history-table", DataTable)
        for col in _COLS:
            table.add_column(col, key=col.lower())
        self._poll()
        self.set_interval(self.poll_interval, self._poll)

    def on_select_changed(self, event: Select.Changed) -> None:
        if event.select.id == "history-window-select" and event.value:
            self._history_window = str(event.value)
            self._poll()

    def _poll(self) -> None:
        self.run_worker(self._fetch, exclusive=True, group="history-fetch")

    async def _fetch(self) -> None:
        loop = asyncio.get_running_loop()
        user = self.slurm.current_user()
        window = self._history_window
        rows = await loop.run_in_executor(
            None, lambda: self.slurm.get_sacct(user=user, start_time=window),
        )
        self._update_table(rows)

        # Find the human-readable label for the current window
        label = window
        for lbl, val in _TIME_WINDOWS:
            if val == window:
                label = lbl
                break

        status = self.query_one("#history-status", Static)
        if not rows:
            status.update(f" No completed jobs found (last {label})")
        else:
            status.update(f" {len(rows)} completed jobs (last {label})")

    def _update_table(self, rows: list[dict[str, str]]) -> None:
        table = self.query_one("#history-table", DataTable)
        table.clear()
        for row in rows:
            state = row.get("state", "")
            color = state_color(state)
            job_id = row.get("job_id", "")
            table.add_row(
                job_id,
                row.get("name", ""),
                row.get("partition", ""),
                f"[{color}]{escape_markup(state)}[/{color}]",
                row.get("elapsed", ""),
                row.get("total_cpu", ""),
                row.get("max_rss", ""),
                row.get("exit_code", ""),
                key=job_id,
            )

    def _get_selected_job_id(self) -> str | None:
        table = self.query_one("#history-table", DataTable)
        if table.cursor_row is not None and table.row_count > 0:
            try:
                row_key = table.coordinate_to_cell_key((table.cursor_row, 0)).row_key
                return str(row_key.value)
            except Exception:
                return None
        return None

    def action_inspect_job(self) -> None:
        job_id = self._get_selected_job_id()
        if job_id:
            self.post_message(self.InspectJobRequested(job_id))

    def action_resubmit(self) -> None:
        job_id = self._get_selected_job_id()
        if job_id:
            self.post_message(self.ResubmitRequested(job_id))

    def on_data_table_row_selected(self, event: DataTable.RowSelected) -> None:
        """Handle Enter on a DataTable row — inspect the job."""
        job_id = self._get_selected_job_id()
        if job_id:
            self.post_message(self.InspectJobRequested(job_id))

    def action_refresh(self) -> None:
        now = time.monotonic()
        if now - self._last_manual_refresh < 2.0:
            return
        self._last_manual_refresh = now
        self._poll()
