"""Job Inspector tab — visual metadata display + live log tailing + real plots."""

from __future__ import annotations

import asyncio

from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Vertical, Horizontal, VerticalScroll
from textual.message import Message
from textual.widgets import Static, RichLog, TabbedContent, TabPane, Label, ProgressBar
from textual.timer import Timer
from textual_plotext import PlotextPlot

from slurm_term.slurm_api import SlurmController
from slurm_term.utils.formatting import escape_markup, state_color

from typing import TYPE_CHECKING
if TYPE_CHECKING:
    from slurm_term.config import SlurmTermConfig

# --- Tunables -----------------------------------------------------------
METRICS_ROLLING_WINDOW = 60        # max data points kept per metric chart
MEMORY_FALLBACK_MB = 64_000        # assumed total memory when not reported


class _MetricChart(PlotextPlot):
    """A single metric chart with built-in data storage."""

    def __init__(self, title: str, color: str, **kwargs) -> None:
        super().__init__(**kwargs)
        self._title = title
        self._color = color
        self._data: list[float] = []

    def on_mount(self) -> None:
        super().on_mount()
        self._redraw()

    def update_data(self, data: list[float]) -> None:
        self._data = list(data)
        self._redraw()

    def _redraw(self) -> None:
        plt = self.plt
        plt.clear_data()
        plt.clear_figure()

        plt.theme("dark")
        plt.canvas_color((30, 30, 40))
        plt.axes_color((30, 30, 40))
        plt.ticks_color((140, 140, 160))

        plt.plot(self._data or [0], color=self._color, marker="braille")
        plt.ylim(0, 100)
        plt.title(self._title)
        plt.xlabel("")
        plt.ylabel("%")

        self.refresh()


def _parse_rss_to_pct(rss_str: str, total_mb: int) -> float:
    """Convert a MaxRSS string like '1234M' or '5G' to a percentage of total."""
    if not rss_str or total_mb <= 0:
        return 0.0
    rss_str = rss_str.strip()
    try:
        if rss_str.endswith("G"):
            mb = float(rss_str[:-1]) * 1024
        elif rss_str.endswith("M"):
            mb = float(rss_str[:-1])
        elif rss_str.endswith("K"):
            mb = float(rss_str[:-1]) / 1024
        else:
            mb = float(rss_str) / (1024 * 1024)  # assume bytes
        return min(100.0, mb / total_mb * 100)
    except (ValueError, TypeError):
        return 0.0


def _parse_duration_to_seconds(duration: str) -> float:
    """Parse a Slurm duration like '[DD-[HH:]]MM:SS[.SSS]' to seconds."""
    if not duration:
        return 0.0
    duration = duration.strip()
    days = 0
    if "-" in duration:
        day_part, duration = duration.split("-", 1)
        try:
            days = int(day_part)
        except ValueError:
            return 0.0
    parts = duration.split(":")
    try:
        if len(parts) == 3:
            h, m, s = int(parts[0]), int(parts[1]), float(parts[2])
        elif len(parts) == 2:
            h, m, s = 0, int(parts[0]), float(parts[1])
        else:
            return 0.0
    except (ValueError, TypeError):
        return 0.0
    return days * 86400 + h * 3600 + m * 60 + s


def _parse_cpu_pct(cpu_str: str, elapsed_seconds: float = 0.0) -> float:
    """Parse an AveCPU string to a CPU-utilisation percentage.

    ``sstat`` returns AveCPU as a *duration* (e.g. ``01:23:45``).
    If *elapsed_seconds* is provided the percentage is calculated as
    ``(avg_cpu_seconds / elapsed) * 100``.

    A trailing ``%`` is also accepted for compatibility.
    """
    if not cpu_str:
        return 0.0
    cpu_str = cpu_str.strip()
    if cpu_str.endswith("%"):
        try:
            return min(100.0, float(cpu_str[:-1]))
        except ValueError:
            return 0.0
    # Treat as Slurm duration
    cpu_seconds = _parse_duration_to_seconds(cpu_str)
    if cpu_seconds > 0 and elapsed_seconds > 0:
        return min(100.0, cpu_seconds / elapsed_seconds * 100)
    return 0.0


class InspectorTab(Vertical):
    """Job inspector: visual metadata + live log viewer + real-time charts."""

    class ResubmitRequested(Message):
        """Posted when the user wants to resubmit the current job."""

        def __init__(self, form_state: dict[str, str]) -> None:
            super().__init__()
            self.form_state = form_state

    BINDINGS = [
        Binding("escape", "back_to_monitor", "Back", show=True),
        Binding("s", "resubmit", "Resubmit", show=True),
        Binding("e", "toggle_log_stream", "Toggle stdout/stderr", show=True),
    ]

    DEFAULT_CSS = """
    InspectorTab {
        height: 1fr;
    }

    #inspector-placeholder {
        width: 100%;
        height: 100%;
        content-align: center middle;
        color: $text-muted;
        text-style: italic;
    }

    #inspector-header {
        height: auto;
        padding: 1 2;
        background: $primary-background;
        border-bottom: tall $accent;
        display: none;
    }
    #inspector-tabs {
        height: 1fr;
        display: none;
    }

    /* Overview tab */
    #tab-overview {
        padding: 1;
    }
    #inspector-meta-grid {
        height: auto;
        padding: 1 2;
        layout: grid;
        grid-size: 2 3;
        grid-columns: 1fr 1fr;
        grid-gutter: 1 3;
        margin-bottom: 1;
    }
    .meta-card {
        height: auto;
        padding: 0;
    }
    #inspector-resources-summary {
        height: auto;
        padding: 1 2;
        background: $surface-darken-1;
        border: round $primary-background;
        margin: 0 1 1 1;
    }
    .section-label {
        color: $accent;
        text-style: bold;
        margin-top: 1;
        margin-bottom: 0;
        padding: 0 1;
    }
    #time-progress-container {
        padding: 0 2;
        height: auto;
        margin-bottom: 1;
    }
    #time-progress {
        width: 100%;
    }

    /* Logs tab */
    #tab-logs {
        padding: 0;
        height: 1fr;
    }
    #log-paths {
        height: auto;
        padding: 1 2;
        background: $surface-darken-1;
        color: $text;
        border-bottom: solid $primary-background;
    }
    #inspector-logs {
        height: 1fr;
        padding: 0 1;
        background: $surface-darken-2;
    }

    /* Metrics tab */
    #tab-metrics {
        padding: 0;
        height: 1fr;
    }
    #metrics-scroll {
        height: 1fr;
    }
    .metric-chart {
        height: 12;
        margin: 0 1;
        border: round $primary-background;
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
        self._current_job_id: str | None = None
        self._tailing: bool = False
        self._tail_gen: int = 0  # incremented on each new tail to stop stale workers
        self._mounted: bool = False
        self._pending_job_id: str | None = None
        self._stdout_path: str = ""
        self._stderr_path: str = ""
        self._log_mode: str = "stdout"  # "stdout" or "stderr"
        self._poll_timer: Timer | None = None
        # Rolling metric histories for sstat-based polling
        self._cpu_history: list[float] = []
        self._mem_history: list[float] = []
        self._gpu_history: list[float] = []

    def compose(self) -> ComposeResult:
        yield Static(
            "Select a job from the Monitor tab and press [bold]Enter[/bold] to inspect it.",
            id="inspector-placeholder",
        )
        yield Static("", id="inspector-header")

        with TabbedContent(id="inspector-tabs"):
            with TabPane("Overview", id="tab-overview"):
                with VerticalScroll():
                    yield Horizontal(id="inspector-meta-grid")
                    yield Static("", id="inspector-resources-summary")
                    yield Label("Time Remaining", classes="section-label")
                    with Vertical(id="time-progress-container"):
                        yield ProgressBar(id="time-progress", show_eta=True)

            with TabPane("Output & Error Logs", id="tab-logs"):
                yield Static(id="log-paths")
                yield RichLog(id="inspector-logs", wrap=True, highlight=True, markup=True)

            with TabPane("Live Metrics", id="tab-metrics"):
                with VerticalScroll(id="metrics-scroll"):
                    yield _MetricChart(
                        "CPU Load", color=(80, 180, 255),
                        id="chart-cpu", classes="metric-chart",
                    )
                    yield _MetricChart(
                        "Memory Usage", color=(120, 220, 120),
                        id="chart-mem", classes="metric-chart",
                    )
                    yield _MetricChart(
                        "GPU Utilization", color=(255, 160, 60),
                        id="chart-gpu", classes="metric-chart",
                    )

        yield Static("", id="inspector-status", classes="status-bar")

    def on_mount(self) -> None:
        self._mounted = True
        if self._pending_job_id:
            jid = self._pending_job_id
            self._pending_job_id = None
            self.load_job(jid)

    def on_unmount(self) -> None:
        self._stop_polling()

    def _stop_polling(self) -> None:
        if self._poll_timer is not None:
            self._poll_timer.stop()
            self._poll_timer = None
        self._tailing = False

    def load_job(self, job_id: str) -> None:
        if not self._mounted:
            self._pending_job_id = job_id
            return

        self._stop_polling()
        self._current_job_id = job_id
        self._cpu_history.clear()
        self._mem_history.clear()
        self._gpu_history.clear()

        self.query_one("#inspector-placeholder").display = False
        self.query_one("#inspector-header").display = True
        self.query_one("#inspector-tabs").display = True

        details = self.slurm.get_job_details(job_id)
        if not details:
            self.query_one("#inspector-header", Static).update(
                f"[bold red]Could not fetch details for job {job_id}[/bold red]"
            )
            self.query_one("#inspector-tabs").display = False
            self._set_status(f"Failed to load job {job_id}")
            return

        self._render_header(details, job_id)
        self._render_metadata(details, job_id)
        self._render_resources(details)
        self._render_paths(details)
        self._start_log_tail(details)
        self._update_metrics(details)
        self._set_status(f"Inspecting job {job_id}")

        self._poll_timer = self.set_interval(self.poll_interval, self._poll_refresh)

    def action_back_to_monitor(self) -> None:
        """Switch back to the Monitor tab and stop background work."""
        self._stop_polling()
        try:
            self.app.action_switch_tab("monitor")
        except AttributeError:
            pass

    def action_resubmit(self) -> None:
        """Extract current job params and request resubmission via Composer."""
        if not self._current_job_id:
            return
        details = self.slurm.get_job_details(self._current_job_id)
        if not details:
            self._set_status("Cannot fetch job details for resubmission")
            return
        form_state = self.extract_form_state(details)
        self.post_message(self.ResubmitRequested(form_state))

    @staticmethod
    def extract_form_state(details: dict) -> dict[str, str]:
        """Convert scontrol job details into a Composer form state dict."""
        state: dict[str, str] = {"mode": "sbatch"}
        state["name"] = str(details.get("name", ""))
        state["partition"] = str(details.get("partition", ""))

        # Time limit (minutes → HH:MM:SS)
        tl = details.get("time_limit", 0)
        if isinstance(tl, dict):
            tl = int(tl.get("number", 0) or 0)
        else:
            tl = int(tl or 0)
        if tl > 0:
            h, rem = divmod(tl * 60, 3600)
            m, s = divmod(rem, 60)
            state["time"] = f"{h:02d}:{m:02d}:{s:02d}"

        nodes = details.get("node_count", "")
        if isinstance(nodes, dict):
            nodes = nodes.get("number", "")
        state["nodes"] = str(nodes) if nodes else "1"

        ntasks = details.get("tasks_per_node", "")
        if isinstance(ntasks, dict):
            ntasks = ntasks.get("number", "")
        state["ntasks"] = str(ntasks) if ntasks else ""

        cpus = details.get("cpus_per_task", "")
        if isinstance(cpus, dict):
            cpus = cpus.get("number", "")
        state["cpus"] = str(cpus) if cpus else ""

        mem = details.get("minimum_memory_per_node", "")
        if isinstance(mem, dict):
            mem = mem.get("number", "")
        if mem:
            try:
                gb = int(mem) / 1024
                state["memory"] = f"{gb:.0f}G" if gb >= 1 else f"{mem}M"
            except (ValueError, TypeError):
                state["memory"] = str(mem)

        gres = details.get("gres_detail", "")
        if gres and str(gres) not in ("(null)", "", "[]"):
            state["gpus"] = str(gres)

        state["script"] = str(details.get("command", ""))
        state["output"] = str(details.get("standard_output", ""))
        state["error"] = str(details.get("standard_error", ""))

        return state

    def _set_status(self, msg: str) -> None:
        self.query_one("#inspector-status", Static).update(f" {msg}")

    def _poll_refresh(self) -> None:
        if not self._current_job_id:
            return
        self.run_worker(self._async_poll_refresh, exclusive=True, group="inspector-poll")

    async def _async_poll_refresh(self) -> None:
        if not self._current_job_id:
            return
        loop = asyncio.get_running_loop()
        details = await loop.run_in_executor(
            None, self.slurm.get_job_details, self._current_job_id,
        )
        if not details:
            return
        self._render_header(details, self._current_job_id)
        self._update_metrics(details)

    def _render_header(self, details: dict, job_id: str) -> None:
        header = self.query_one("#inspector-header", Static)
        name = escape_markup(str(details.get("name", "N/A")))
        state = str(details.get("job_state", "UNKNOWN"))
        color = state_color(state)

        header.update(
            f"[bold]{name}[/bold]  "
            f"[{color} bold] {escape_markup(state)} [/{color} bold]  "
            f"[dim]Job {escape_markup(job_id)}[/dim]"
        )

    def _render_metadata(self, details: dict, job_id: str) -> None:
        grid = self.query_one("#inspector-meta-grid", Horizontal)
        grid.remove_children()

        submit_raw = details.get("submit_time", "N/A")
        if isinstance(submit_raw, dict):
            ts = submit_raw.get("number", "")
            if ts:
                try:
                    from datetime import datetime, timezone
                    submit_time = datetime.fromtimestamp(
                        int(ts), tz=timezone.utc,
                    ).strftime("%Y-%m-%d %H:%M:%S UTC")
                except (ValueError, TypeError, OSError):
                    submit_time = str(ts)
            else:
                submit_time = "N/A"
        else:
            submit_time = str(submit_raw) if submit_raw else "N/A"

        fields = [
            ("Job ID", job_id),
            ("Partition", details.get("partition", "N/A")),
            ("User", details.get("user_name", "N/A")),
            ("Submit Time", submit_time),
            ("Work Dir", details.get("working_directory", "N/A")),
            ("Nodes", str(details.get("nodes", "N/A"))),
        ]
        for label, value in fields:
            grid.mount(
                Static(
                    f"[bold cyan]{label}:[/bold cyan] {escape_markup(str(value))}",
                    markup=True, classes="meta-card",
                )
            )

    def _render_paths(self, details: dict) -> None:
        paths = self.query_one("#log-paths", Static)
        stdout = escape_markup(str(details.get("standard_output", "N/A")))
        stderr = escape_markup(str(details.get("standard_error", "N/A")))
        paths.update(
            f"[bold cyan]stdout:[/bold cyan] {stdout}\n"
            f"[bold red]stderr:[/bold red] {stderr}"
        )

    def _render_resources(self, details: dict) -> None:
        res = self.query_one("#inspector-resources-summary", Static)
        parts = []

        nodes = details.get("nodes", "?")
        if isinstance(nodes, dict):
            nodes = nodes.get("number", "?")
        parts.append(f"[bold]{nodes}[/bold] node(s)")

        cpus = details.get("cpus_per_task", "")
        if cpus:
            parts.append(f"[bold]{cpus}[/bold] CPUs/task")

        mem = details.get("minimum_memory_per_node", "")
        if mem:
            try:
                mb = int(mem)
                parts.append(f"[bold]{mb / 1024:.1f}[/bold] GB")
            except (ValueError, TypeError):
                parts.append(f"[bold]{mem}[/bold]")

        gres = details.get("gres_detail", "")
        if gres and str(gres) not in ("(null)", "", "[]"):
            parts.append(f"[bold]{gres}[/bold]")

        res.update(" × ".join(parts) if parts else "[dim]No resource details[/dim]")

    def _update_metrics(self, details: dict) -> None:
        state = str(details.get("job_state", "UNKNOWN")).upper()

        # ---- Time Progress ----
        # time_limit is in minutes; may be a plain int or a
        # v0.0.44_uint32_no_val_struct dict like {"number": N, "set": true, "infinite": false}
        tl_raw = details.get("time_limit", 0)
        if isinstance(tl_raw, dict):
            total_minutes = int(tl_raw.get("number", 0) or 0)
        else:
            total_minutes = int(tl_raw or 0)
        total_time = total_minutes * 60  # convert to seconds

        # run_time may not exist in some Slurm versions —
        # fall back to computing from start_time.
        rt_raw = details.get("run_time", None)
        if rt_raw is not None:
            if isinstance(rt_raw, dict):
                run_time = int(rt_raw.get("number", 0) or 0)
            else:
                run_time = int(rt_raw or 0)
        else:
            st_raw = details.get("start_time", 0)
            if isinstance(st_raw, dict):
                start_ts = int(st_raw.get("number", 0) or 0)
            else:
                start_ts = int(st_raw or 0)
            if start_ts > 0:
                import time as _time
                run_time = max(0, int(_time.time()) - start_ts)
            else:
                run_time = 0

        try:
            if total_time > 0:
                prog = self.query_one("#time-progress", ProgressBar)
                prog.total = total_time
                prog.progress = min(run_time, total_time)
        except (ValueError, TypeError, LookupError):
            pass

        # If mock metrics are available (demo mode), use them directly
        mock_metrics = details.get("slurmterm_metrics", {})
        if mock_metrics:
            for chart_id, metric_key in [
                ("chart-cpu", "cpu"),
                ("chart-mem", "mem"),
                ("chart-gpu", "gpu"),
            ]:
                try:
                    chart = self.query_one(f"#{chart_id}", _MetricChart)
                    chart.update_data(mock_metrics.get(metric_key, [0.0]))
                except LookupError:
                    pass
            return

        # Real mode: poll sstat for live metrics
        if state != "RUNNING" or not self._current_job_id:
            return
        sstat = self.slurm.get_sstat(self._current_job_id)
        if not sstat:
            return

        total_mem_mb = int(details.get("minimum_memory_per_node", 0) or 0)
        if total_mem_mb <= 0:
            total_mem_mb = MEMORY_FALLBACK_MB

        # AveCPU is a duration — compute % relative to wall-clock elapsed
        elapsed_sec = float(run_time) if run_time > 0 else 0.0
        cpu_val = _parse_cpu_pct(sstat.get("avg_cpu", ""), elapsed_sec)
        mem_val = _parse_rss_to_pct(sstat.get("max_rss", ""), total_mem_mb)

        # GPU metrics: use nvidia-smi if enabled in config
        gpu_val = 0.0
        if (self._config and self._config.gpu_monitor_enabled
                and "gpu" in str(details.get("gres_detail", "")).lower()):
            node = str(details.get("nodes", "")) or None
            gpu_vals = self.slurm.get_gpu_utilization(node=node)
            if gpu_vals:
                gpu_val = sum(gpu_vals) / len(gpu_vals)

        self._cpu_history.append(cpu_val)
        self._mem_history.append(mem_val)
        self._gpu_history.append(gpu_val)

        # Keep rolling window
        for hist in (self._cpu_history, self._mem_history, self._gpu_history):
            while len(hist) > METRICS_ROLLING_WINDOW:
                hist.pop(0)

        for chart_id, data in [
            ("chart-cpu", self._cpu_history),
            ("chart-mem", self._mem_history),
            ("chart-gpu", self._gpu_history),
        ]:
            try:
                self.query_one(f"#{chart_id}", _MetricChart).update_data(data)
            except LookupError:
                pass

    def _start_log_tail(self, details: dict) -> None:
        self._stdout_path = details.get("standard_output", "")
        self._stderr_path = details.get("standard_error", "")
        self._log_mode = "stdout"
        log_widget = self.query_one("#inspector-logs", RichLog)
        log_widget.clear()

        self._tailing = False
        self._tail_gen += 1
        gen = self._tail_gen

        path = self._stdout_path
        if path:
            log_widget.write(f"[dim]── Tailing stdout: {escape_markup(path)} ──[/dim]\n")
            if self._stderr_path:
                log_widget.write("[dim]Press [bold]e[/bold] to switch to stderr[/dim]\n")
            self.run_worker(
                lambda: self._tail_file(path, gen),
                exclusive=True, group="log-tail", thread=True,
            )
        else:
            log_widget.write("[dim]No stdout file found for this job.[/dim]")

    def action_toggle_log_stream(self) -> None:
        """Switch between tailing stdout and stderr."""
        if self._log_mode == "stdout" and self._stderr_path:
            self._log_mode = "stderr"
            path = self._stderr_path
        elif self._log_mode == "stderr" and self._stdout_path:
            self._log_mode = "stdout"
            path = self._stdout_path
        else:
            return

        log_widget = self.query_one("#inspector-logs", RichLog)
        log_widget.clear()
        self._tailing = False
        self._tail_gen += 1
        gen = self._tail_gen

        other = "stderr" if self._log_mode == "stdout" else "stdout"
        log_widget.write(f"[dim]── Tailing {self._log_mode}: {escape_markup(path)} ──[/dim]\n")
        log_widget.write(f"[dim]Press [bold]e[/bold] to switch to {other}[/dim]\n")
        self.run_worker(
            lambda: self._tail_file(path, gen),
            exclusive=True, group="log-tail", thread=True,
        )
        self._set_status(f"Viewing {self._log_mode}")

    # Max bytes to read on initial log load (1 MB)
    _MAX_INITIAL_READ = 1024 * 1024

    def _tail_file(self, path: str, gen: int) -> None:
        import os
        import time
        from textual.worker import get_current_worker

        worker = get_current_worker()
        self._tailing = True
        try:
            with open(path, "r", errors="replace") as f:
                # For large files, seek to the last _MAX_INITIAL_READ bytes
                try:
                    size = os.fstat(f.fileno()).st_size
                    if size > self._MAX_INITIAL_READ:
                        f.seek(size - self._MAX_INITIAL_READ)
                        f.readline()  # skip partial first line
                        self.app.call_from_thread(
                            self._append_log, f"[dim]… (skipped {size - self._MAX_INITIAL_READ} bytes)[/dim]\n"
                        )
                except OSError:
                    pass

                content = f.read()
                if content:
                    self.app.call_from_thread(self._append_log, content)

                while not worker.is_cancelled and self._tailing and self._tail_gen == gen:
                    try:
                        line = f.readline()
                    except OSError:
                        self.app.call_from_thread(
                            self._append_log,
                            "[red]Log file became inaccessible[/red]\n",
                        )
                        break
                    if line:
                        self.app.call_from_thread(self._append_log, line)
                    else:
                        time.sleep(0.5)
        except FileNotFoundError:
            self.app.call_from_thread(
                self._append_log, f"[red]File not found: {path}[/red]\n"
            )
        except PermissionError:
            self.app.call_from_thread(
                self._append_log, f"[red]Permission denied: {path}[/red]\n"
            )

    def _append_log(self, text: str) -> None:
        try:
            self.query_one("#inspector-logs", RichLog).write(text)
        except LookupError:
            pass
