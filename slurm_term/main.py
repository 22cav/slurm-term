"""SlurmTerm — main application entry point."""

from __future__ import annotations

from pathlib import Path

from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.widgets import Header, Footer, TabbedContent, TabPane

from slurm_term.config import SlurmTermConfig, load_config
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
        Binding("1", "switch_tab('monitor')", "Monitor", show=False),
        Binding("2", "switch_tab('composer')", "Composer", show=False),
        Binding("3", "switch_tab('hardware')", "Hardware", show=False),
        Binding("4", "switch_tab('history')", "History", show=False),
        Binding("5", "switch_tab('inspector')", "Inspector", show=False),
        Binding("ctrl+r", "reload_config", "Reload Config", show=False),
    ]

    def __init__(
        self,
        slurm: SlurmController | None = None,
        config: SlurmTermConfig | None = None,
        **kwargs,
    ) -> None:
        super().__init__(**kwargs)
        self.config = config or SlurmTermConfig()
        self.slurm = slurm or SlurmController()
        self._inspector: InspectorTab | None = None
        self._monitor: MonitorTab | None = None

    def compose(self) -> ComposeResult:
        yield Header(show_clock=True)
        with TabbedContent(id="tabs"):
            with TabPane("Monitor", id="monitor"):
                self._monitor = MonitorTab(
                    slurm=self.slurm,
                    poll_interval=self.config.monitor_poll_interval,
                    config=self.config,
                )
                yield self._monitor
            with TabPane("Composer", id="composer"):
                yield ComposerTab(slurm=self.slurm)
            with TabPane("Hardware", id="hardware"):
                yield HardwareTab(
                    slurm=self.slurm,
                    poll_interval=self.config.hardware_poll_interval,
                )
            with TabPane("History", id="history"):
                yield HistoryTab(
                    slurm=self.slurm,
                    poll_interval=self.config.history_poll_interval,
                    history_window=self.config.history_window,
                )
            with TabPane("Inspector", id="inspector"):
                self._inspector = InspectorTab(
                    slurm=self.slurm,
                    poll_interval=self.config.inspector_poll_interval,
                    config=self.config,
                )
                yield self._inspector
        yield Footer()

    def on_mount(self) -> None:
        try:
            cluster = self.slurm.get_cluster_name()
            user = self.slurm.current_user()
            self.sub_title = f"{cluster} • {user}"
        except Exception:
            self.log.warning("Could not fetch cluster name")
            self.sub_title = self.slurm.current_user()

    def action_reload_config(self) -> None:
        """Reload configuration from disk."""
        from dataclasses import fields
        new_cfg = load_config()
        for f in fields(new_cfg):
            setattr(self.config, f.name, getattr(new_cfg, f.name))
        self.notify("Configuration reloaded", severity="information")

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

    def on_inspector_tab_resubmit_requested(
        self, event: InspectorTab.ResubmitRequested,
    ) -> None:
        """Pre-populate Composer with job params and switch to it."""
        self._load_composer_state(event.form_state)

    async def on_history_tab_resubmit_requested(
        self, event: HistoryTab.ResubmitRequested,
    ) -> None:
        """Fetch job details, pre-populate Composer, and switch to it."""
        import asyncio

        loop = asyncio.get_running_loop()
        details = await loop.run_in_executor(
            None, self.slurm.get_job_details, event.job_id,
        )
        if not details:
            self.notify(
                f"Could not fetch details for job {event.job_id}",
                severity="error",
            )
            return
        form_state = InspectorTab.extract_form_state(details)
        self._load_composer_state(form_state)

    def _load_composer_state(self, form_state: dict[str, str]) -> None:
        """Switch to Composer tab and load form state."""
        self.action_switch_tab("composer")
        try:
            composer = self.query_one(ComposerTab)
            composer.set_form_state(form_state)
            composer.set_status("Job parameters loaded for resubmission")
        except Exception:
            self.notify(
                "Failed to load job parameters into Composer",
                severity="error",
            )


def run() -> None:
    """CLI entry point."""
    import argparse

    from slurm_term import __version__

    parser = argparse.ArgumentParser(description="SlurmTerm — TUI for Slurm")
    parser.add_argument(
        "--version", action="version", version=f"%(prog)s {__version__}",
    )
    parser.add_argument(
        "--demo", action="store_true",
        help="Run with a simulated cluster (no real Slurm needed)",
    )
    parser.add_argument(
        "--since",
        help="Initial history window (e.g. 'now-3days', '2026-01-01')",
    )
    args = parser.parse_args()

    cfg = load_config()
    if args.since:
        cfg.history_window = args.since

    from slurm_term.default_templates import ensure_default_templates
    ensure_default_templates()

    slurm: SlurmController | None = None
    if args.demo:
        from slurm_term.mock_slurm import MockSlurmController
        slurm = MockSlurmController()

    SlurmTermApp(slurm=slurm, config=cfg).run()


if __name__ == "__main__":
    run()
