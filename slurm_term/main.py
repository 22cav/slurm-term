"""SlurmTerm — main application entry point."""

from __future__ import annotations

from pathlib import Path

from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.widgets import Header, Footer, TabbedContent, TabPane

from slurm_term.slurm_api import SlurmController
from slurm_term.screens.monitor import MonitorTab
from slurm_term.screens.composer import ComposerTab
from slurm_term.screens.inspector import InspectorTab
from slurm_term.screens.hardware import HardwareTab
from slurm_term.screens.history import HistoryTab


CSS_PATH = Path(__file__).parent / "layout.css"


class SlurmTermApp(App):
    """A Terminal User Interface for the Slurm workload manager."""

    TITLE = "SlurmTerm"
    SUB_TITLE = ""
    CSS_PATH = CSS_PATH

    BINDINGS = [
        Binding("q", "quit", "Quit", show=True, priority=True),
        Binding("1", "switch_tab('monitor')", "Monitor", show=True),
        Binding("2", "switch_tab('composer')", "Composer", show=True),
        Binding("3", "switch_tab('hardware')", "Hardware", show=True),
        Binding("4", "switch_tab('history')", "History", show=True),
        Binding("5", "switch_tab('inspector')", "Inspector", show=True),
    ]

    def __init__(self, slurm: SlurmController | None = None, **kwargs) -> None:
        super().__init__(**kwargs)
        self.slurm = slurm or SlurmController()
        self._inspector: InspectorTab | None = None
        self._monitor: MonitorTab | None = None

    def compose(self) -> ComposeResult:
        yield Header(show_clock=True)
        with TabbedContent(id="tabs"):
            with TabPane("Monitor", id="monitor"):
                self._monitor = MonitorTab(slurm=self.slurm)
                yield self._monitor
            with TabPane("Composer", id="composer"):
                yield ComposerTab(slurm=self.slurm)
            with TabPane("Hardware", id="hardware"):
                yield HardwareTab(slurm=self.slurm)
            with TabPane("History", id="history"):
                yield HistoryTab(slurm=self.slurm)
            with TabPane("Inspector", id="inspector"):
                self._inspector = InspectorTab(slurm=self.slurm)
                yield self._inspector
        yield Footer()

    def on_mount(self) -> None:
        try:
            cluster = self.slurm.get_cluster_name()
            user = self.slurm.current_user()
            self.sub_title = f"{cluster} • {user}"
        except Exception:
            self.sub_title = self.slurm.current_user()

    def action_switch_tab(self, tab_id: str) -> None:
        self.query_one("#tabs", TabbedContent).active = tab_id

    def inspect_job(self, job_id: str) -> None:
        self.action_switch_tab("inspector")
        if self._inspector:
            self._inspector.load_job(job_id)

    def on_monitor_tab_inspect_job_requested(
        self, event: MonitorTab.InspectJobRequested,
    ) -> None:
        """Handle inspect request from MonitorTab."""
        self.inspect_job(event.job_id)

    def on_history_tab_inspect_job_requested(
        self, event: HistoryTab.InspectJobRequested,
    ) -> None:
        """Handle inspect request from HistoryTab."""
        self.inspect_job(event.job_id)


def run() -> None:
    """CLI entry point."""
    import argparse

    parser = argparse.ArgumentParser(description="SlurmTerm — TUI for Slurm")
    parser.add_argument(
        "--demo", action="store_true",
        help="Run with a simulated cluster (no real Slurm needed)",
    )
    args = parser.parse_args()

    slurm: SlurmController | None = None
    if args.demo:
        from slurm_term.mock_slurm import MockSlurmController
        slurm = MockSlurmController()

    SlurmTermApp(slurm=slurm).run()


if __name__ == "__main__":
    run()
