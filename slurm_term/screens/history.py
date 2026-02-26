"""History / Accounting tab — completed jobs from sacct."""

from __future__ import annotations

import asyncio

from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Vertical
from textual.message import Message
from textual.widgets import DataTable, Static

from slurm_term.slurm_api import SlurmController
from slurm_term.utils.formatting import escape_markup, state_color

_COLS = ("JobID", "Name", "Partition", "State", "Elapsed", "TotalCPU", "MaxRSS", "Exit")


class HistoryTab(Vertical):
    """Completed job history from sacct."""

    class InspectJobRequested(Message):
        """Posted when the user wants to inspect a completed job."""

        def __init__(self, job_id: str) -> None:
            super().__init__()
            self.job_id = job_id

    BINDINGS = [
        Binding("r", "refresh", "Refresh", show=True),
        Binding("i", "inspect_job", "Inspect", show=True),
    ]

    DEFAULT_CSS = """
    HistoryTab {
        height: 1fr;
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
        **kwargs,
    ) -> None:
        super().__init__(**kwargs)
        self.slurm = slurm or SlurmController()
        self.poll_interval = poll_interval

    def compose(self) -> ComposeResult:
        yield DataTable(id="history-table", cursor_type="row")
        yield Static("Loading job history…", id="history-status", classes="status-bar")

    def on_mount(self) -> None:
        table = self.query_one("#history-table", DataTable)
        for col in _COLS:
            table.add_column(col, key=col.lower())
        self._poll()
        self.set_interval(self.poll_interval, self._poll)

    def _poll(self) -> None:
        self.run_worker(self._fetch, exclusive=True, group="history-fetch")

    async def _fetch(self) -> None:
        loop = asyncio.get_running_loop()
        user = self.slurm.current_user()
        rows = await loop.run_in_executor(
            None, lambda: self.slurm.get_sacct(user=user, start_time="now-7days"),
        )
        self._update_table(rows)
        status = self.query_one("#history-status", Static)
        if not rows:
            status.update(" No completed jobs found (last 7 days)")
        else:
            status.update(f" {len(rows)} completed jobs (last 7 days)")

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

    def on_data_table_row_selected(self, event: DataTable.RowSelected) -> None:
        """Handle Enter on a DataTable row — inspect the job."""
        job_id = self._get_selected_job_id()
        if job_id:
            self.post_message(self.InspectJobRequested(job_id))

    def action_refresh(self) -> None:
        self._poll()
