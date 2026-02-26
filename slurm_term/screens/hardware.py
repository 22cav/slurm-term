"""Hardware / Cluster Info tab — partition summary + node details."""

from __future__ import annotations

import asyncio

from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Vertical
from textual.widgets import DataTable, Static, TabbedContent, TabPane

from slurm_term.slurm_api import SlurmController


_PART_COLS = ("Partition", "Avail", "TimeLimit", "Nodes", "State", "CPUs", "Mem(GB)", "GRES")
_NODE_COLS = ("Node", "State", "CPUs", "Mem(GB)", "GRES", "Partitions", "Load", "Free(GB)")


def _mb_to_gb(val: str) -> str:
    """Convert an MB string to GB with 1 decimal."""
    try:
        return f"{int(val) / 1024:.1f}"
    except (ValueError, TypeError):
        return val


class HardwareTab(Vertical):
    """Cluster hardware overview: partitions + node details."""

    BINDINGS = [
        Binding("r", "refresh", "Refresh", show=True),
    ]

    DEFAULT_CSS = """
    HardwareTab {
        height: 1fr;
    }
    #hw-tabs {
        height: 1fr;
    }
    #hw-part-table, #hw-node-table {
        height: 1fr;
        scrollbar-size: 1 1;
    }
    """

    def __init__(
        self,
        slurm: SlurmController | None = None,
        poll_interval: float = 30.0,
        **kwargs,
    ) -> None:
        super().__init__(**kwargs)
        self.slurm = slurm or SlurmController()
        self.poll_interval = poll_interval

    def compose(self) -> ComposeResult:
        with TabbedContent(id="hw-tabs"):
            with TabPane("Partitions", id="hw-partitions"):
                yield DataTable(id="hw-part-table", cursor_type="row")
            with TabPane("Nodes", id="hw-nodes"):
                yield DataTable(id="hw-node-table", cursor_type="row")
        yield Static("Loading cluster info…", id="hw-status", classes="status-bar")

    def on_mount(self) -> None:
        part_table = self.query_one("#hw-part-table", DataTable)
        for col in _PART_COLS:
            part_table.add_column(col, key=col.lower())

        node_table = self.query_one("#hw-node-table", DataTable)
        for col in _NODE_COLS:
            node_table.add_column(col, key=col.lower())

        self._poll()
        self.set_interval(self.poll_interval, self._poll)

    def _poll(self) -> None:
        self.run_worker(self._fetch, exclusive=True, group="hw-fetch")

    async def _fetch(self) -> None:
        loop = asyncio.get_running_loop()
        sinfo_rows, node_rows = await asyncio.gather(
            loop.run_in_executor(None, self.slurm.get_sinfo),
            loop.run_in_executor(None, self.slurm.get_node_info),
        )
        self._update_partitions(sinfo_rows)
        self._update_nodes(node_rows)
        status = self.query_one("#hw-status", Static)
        if not sinfo_rows and not node_rows:
            status.update(" No cluster data available — check Slurm connectivity")
        else:
            status.update(f" {len(sinfo_rows)} partition entries, {len(node_rows)} nodes")

    def _update_partitions(self, rows: list[dict[str, str]]) -> None:
        table = self.query_one("#hw-part-table", DataTable)
        table.clear()
        for row in rows:
            table.add_row(
                row.get("partition", ""),
                row.get("avail", ""),
                row.get("timelimit", ""),
                row.get("nodes", ""),
                row.get("state", ""),
                row.get("cpus", ""),
                _mb_to_gb(row.get("memory", "")),
                row.get("gres", ""),
            )

    def _update_nodes(self, nodes: list[dict[str, str]]) -> None:
        table = self.query_one("#hw-node-table", DataTable)
        table.clear()
        for node in nodes:
            table.add_row(
                node.get("NodeName", ""),
                node.get("State", ""),
                node.get("CPUTot", ""),
                _mb_to_gb(node.get("RealMemory", "")),
                node.get("Gres", "(none)"),
                node.get("Partitions", ""),
                node.get("CPULoad", "N/A"),
                _mb_to_gb(node.get("FreeMem", "N/A")),
            )

    def action_refresh(self) -> None:
        self._poll()
