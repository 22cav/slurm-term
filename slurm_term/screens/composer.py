"""Unified Composer tab — sbatch / srun script builder with dynamic parameters."""

from __future__ import annotations

import asyncio
import os
import re
from itertools import count

from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Vertical, Horizontal, VerticalScroll
from textual.events import Click
from textual.message import Message
from textual.timer import Timer
from textual.widgets import Static, Input, Select, Button, TextArea, RichLog, Label, Rule

from slurm_term.slurm_api import SlurmController
from slurm_term.utils.formatting import escape_markup
from slurm_term.utils.validators import parse_time, parse_memory, validate_job_name
from slurm_term.screens.param_catalog import FLAG_PARAMS

_GPU_SPEC_RE = re.compile(r"^[a-zA-Z0-9_]*:?\d+$")

# --- Tunables -----------------------------------------------------------
PREVIEW_DEBOUNCE_SEC = 0.3         # delay before refreshing script preview


class ComposerTab(Vertical):
    """Unified job submission form (sbatch + srun) with live preview."""

    class JobSubmitted(Message):
        """Posted when a job is successfully submitted."""

        def __init__(self, job_id: str) -> None:
            super().__init__()
            self.job_id = job_id

    BINDINGS = [
        Binding("ctrl+s", "submit", "Submit / Launch", show=True),
        Binding("ctrl+t", "save_template", "Save Template", show=True),
        Binding("ctrl+l", "load_template", "Load Template", show=True),
        Binding("ctrl+i", "import_sbatch", "Import .sbatch", show=True),
        Binding("ctrl+y", "copy_preview", "Copy Script", show=True),
    ]

    DEFAULT_CSS = """
    ComposerTab {
        height: 1fr;
    }

    /* ---- Grid layout ---- */
    #composer-grid {
        height: 1fr;
        layout: grid;
        grid-size: 2 1;
        grid-columns: 1fr 1fr;
        grid-gutter: 0 2;
        padding: 1 2;
    }
    #composer-form {
        height: 1fr;
        padding: 0;
    }
    #composer-preview-pane {
        height: 1fr;
    }
    #preview-title {
        text-style: bold;
        color: $text;
        background: $primary-background;
        width: 100%;
        height: 1;
        padding: 0 1;
    }
    #composer-preview {
        height: 1fr;
        border: solid $primary-background;
    }

    /* ---- Section titles ---- */
    .section-title {
        text-style: bold;
        color: $accent;
        width: 100%;
        margin-top: 1;
        margin-bottom: 0;
    }
    .section-title-first {
        text-style: bold;
        color: $accent;
        width: 100%;
        margin-bottom: 0;
    }
    .section-rule {
        color: $primary-background;
        margin-bottom: 0;
    }

    /* ---- Form field labels ---- */
    .form-label-row {
        height: 1;
        margin-top: 1;
    }
    .form-label-text {
        text-style: bold;
        color: $text;
        width: 1fr;
    }
    .help-link {
        width: auto;
        color: $text-muted;
        text-style: none;
    }
    .help-link:hover {
        color: $accent;
    }

    /* ---- Inputs ---- */
    Input {
        margin-bottom: 0;
    }
    Input.-invalid {
        border: tall $error;
    }
    .partition-summary {
        color: $text-muted;
        text-style: italic;
        height: auto;
        padding: 0 1;
    }
    #input-modules, #input-env {
        height: 4;
    }
    #input-init {
        height: 6;
    }

    /* ---- Button groups ---- */
    .btn-group {
        height: auto;
        margin-top: 1;
    }
    .btn-group Button {
        width: 1fr;
        margin: 0 1 0 0;
    }
    .btn-group Button:last-of-type {
        margin: 0;
    }
    .btn-full {
        width: 100%;
        margin-top: 1;
    }
    #btn-submit {
        width: 100%;
        margin-top: 2;
    }
    #btn-add-param {
        width: 100%;
        margin-top: 1;
    }

    /* ---- Extra params ---- */
    #extras-container {
        height: auto;
    }
    .extra-row {
        height: auto;
    }
    .extra-header {
        height: 1;
        margin-top: 1;
    }
    .extra-label {
        width: 1fr;
        text-style: bold;
        color: $accent;
    }
    .extra-link {
        width: auto;
        color: $text-muted;
        margin: 0 1;
    }
    .extra-rm {
        width: auto;
        color: $error;
        text-style: bold;
    }
    .extra-input {
        width: 100%;
    }
    .extra-flag-note {
        color: $text-muted;
        text-style: italic;
    }

    /* ---- Hidden section marker ---- */
    .form-section {
        margin-top: 1;
        text-style: bold italic;
        color: $accent;
    }
    """

    def __init__(self, slurm: SlurmController | None = None, **kwargs) -> None:
        super().__init__(**kwargs)
        self.slurm = slurm or SlurmController()
        self._extra_params: list[tuple[str, str, str, str]] = []
        self._uid_to_key: dict[int, str] = {}
        self._uid_counter = count()
        self._preview_timer: Timer | None = None
        self._sinfo_cache: list[dict[str, str]] = []
        self._last_preview_text: str = ""

    def compose(self) -> ComposeResult:
        with Horizontal(id="composer-grid"):
            with VerticalScroll(id="composer-form"):
                # ── Mode ──
                yield Static("Mode", classes="section-title-first")
                yield Select(
                    [("sbatch (Batch Job)", "sbatch"), ("srun (Interactive)", "srun")],
                    value="sbatch", id="select-mode",
                )

                # ── Resources ──
                yield Rule(line_style="heavy", classes="section-rule")
                yield Static("Resources", classes="section-title")

                yield self._label_row("Partition", "partition")
                yield Select([], id="select-partition", prompt="Select partition")
                yield Static("", id="partition-summary", classes="partition-summary")

                yield self._label_row("Time Limit", "time")
                yield Input(value="01:00:00", placeholder="HH:MM:SS", id="input-time")

                yield self._label_row("Nodes", "nodes")
                yield Input(value="1", id="input-nodes")

                yield self._label_row("Tasks / Node", "ntasks-per-node")
                yield Input(value="1", id="input-ntasks")

                yield self._label_row("CPUs / Task", "cpus-per-task")
                yield Input(value="1", id="input-cpus")

                yield self._label_row("Memory", "mem")
                yield Input(value="4G", placeholder="e.g. 4G, 512M", id="input-memory")

                yield self._label_row("GPUs", "gres")
                yield Input(placeholder="e.g. 1 or a100:2", id="input-gpus")

                # ── Batch Job ──
                yield Rule(line_style="heavy", id="rule-batch", classes="section-rule")
                yield Static("Batch Job", classes="section-title", id="section-batch")

                yield self._label_row("Job Name", "job-name", lbl_id="lbl-name")
                yield Input(value="my_job", id="input-name")

                yield self._label_row("Script Path", "_script-path", lbl_id="lbl-script")
                yield Input(placeholder="/path/to/script.sh (optional with init commands)", id="input-script")

                yield self._label_row("Output Pattern", "output", lbl_id="lbl-output")
                yield Input(value="%x-%j.out", id="input-output")

                yield self._label_row("Error Pattern", "error", lbl_id="lbl-error")
                yield Input(value="%x-%j.err", id="input-error")

                # ── Environment ──
                yield Rule(line_style="heavy", id="rule-env", classes="section-rule")
                yield Static("Environment", classes="section-title", id="section-env")

                yield self._label_row("Module Loads", "_modules", lbl_id="lbl-modules")
                yield TextArea(id="input-modules", language=None, soft_wrap=True)

                yield self._label_row("Env Vars", "_env-vars", lbl_id="lbl-env")
                yield TextArea(id="input-env", language=None, soft_wrap=True)

                yield self._label_row("Init Commands", "_init-cmds", lbl_id="lbl-init")
                yield TextArea(id="input-init", language="bash", soft_wrap=True)

                # ── Additional Parameters ──
                yield Rule(line_style="heavy", classes="section-rule")
                yield Static("Additional Parameters", classes="section-title")
                yield Vertical(id="extras-container")
                yield Button("+ Add Parameter…", variant="primary", id="btn-add-param")

                # ── Templates ──
                yield Rule(line_style="heavy", classes="section-rule")
                yield Static("Templates", classes="section-title")
                with Horizontal(classes="btn-group"):
                    yield Button("Save  (^T)", variant="primary", id="btn-save-tmpl")
                    yield Button("Load  (^L)", variant="default", id="btn-load-tmpl")
                    yield Button("Import .sbatch  (^I)", variant="default", id="btn-import-sbatch")

                # ── Submit ──
                yield Button("Submit Job  (^S)", variant="success", id="btn-submit")

            # ── Preview Pane ──────────────────────────────────
            with Vertical(id="composer-preview-pane"):
                yield Static("Script Preview", id="preview-title")
                yield RichLog(id="composer-preview", wrap=True, highlight=True, markup=True)

        yield Static("Fill in the form and press Submit", id="composer-status", classes="status-bar")

    @staticmethod
    def _label_row(text: str, param_key: str, lbl_id: str | None = None) -> Horizontal:
        """Create a single-line label with a clickable [?] help indicator."""
        row_kwargs: dict[str, str] = {}
        if lbl_id:
            row_kwargs["id"] = lbl_id
        row = Horizontal(classes="form-label-row", **row_kwargs)
        row.compose_add_child(Static(text, classes="form-label-text"))
        row.compose_add_child(Static(" ?", classes="help-link", id=f"help-{param_key}"))
        return row

    def on_mount(self) -> None:
        self._update_mode_visibility()
        self._update_preview()
        self.run_worker(self._fetch_partitions, group="partitions")

    async def _fetch_partitions(self) -> None:
        loop = asyncio.get_running_loop()
        parts, sinfo = await asyncio.gather(
            loop.run_in_executor(None, self.slurm.get_partitions),
            loop.run_in_executor(None, self.slurm.get_sinfo),
        )
        self._sinfo_cache = sinfo or []
        if parts:
            sel = self.query_one("#select-partition", Select)
            sel.set_options([(p, p) for p in parts])
            sel.value = parts[0]
            self._update_partition_summary()

    # ---- Mode toggle ------------------------------------------------------

    def _is_srun(self) -> bool:
        return self._sel("select-mode") == "srun"

    def _update_mode_visibility(self) -> None:
        srun = self._is_srun()
        sbatch_ids = [
            "section-batch", "rule-batch", "lbl-name", "input-name",
            "lbl-script", "input-script",
            "lbl-output", "input-output",
            "lbl-error", "input-error",
            "section-env", "rule-env", "lbl-modules", "input-modules",
            "lbl-env", "input-env", "lbl-init", "input-init",
        ]
        for wid in sbatch_ids:
            try:
                self.query_one(f"#{wid}").display = not srun
            except LookupError:
                pass
        try:
            btn = self.query_one("#btn-submit", Button)
            btn.label = "Launch Interactive  (^S)" if srun else "Submit Job  (^S)"
        except LookupError:
            pass

    # ---- Event handlers ---------------------------------------------------

    def _update_partition_summary(self) -> None:
        """Show a one-line resource summary for the selected partition."""
        part = self._sel("select-partition")
        try:
            summary = self.query_one("#partition-summary", Static)
        except LookupError:
            return
        if not part or not self._sinfo_cache:
            summary.update("")
            return
        rows = [r for r in self._sinfo_cache if r.get("partition") == part]
        if not rows:
            summary.update("")
            return
        total_nodes = sum(int(r.get("nodes", "0")) for r in rows)
        cpus = rows[0].get("cpus", "?")
        mem_mb = rows[0].get("memory", "0")
        try:
            mem_gb = f"{int(mem_mb) / 1024:.0f}"
        except (ValueError, TypeError):
            mem_gb = mem_mb
        timelimit = rows[0].get("timelimit", "?")
        gres = rows[0].get("gres", "(null)")
        info = [f"{total_nodes} nodes", f"{cpus} CPUs/node", f"{mem_gb} GB", f"max {timelimit}"]
        if gres and gres != "(null)":
            info.append(gres)
        summary.update(f"[dim]{' \u00b7 '.join(info)}[/dim]")

    def on_input_changed(self, event: Input.Changed) -> None:
        self._schedule_preview()
        self._validate_field(event.input)

    def on_select_changed(self, event: Select.Changed) -> None:
        if event.select.id == "select-mode":
            self._update_mode_visibility()
        if event.select.id == "select-partition":
            self._update_partition_summary()
        self._schedule_preview()

    def on_text_area_changed(self, event: TextArea.Changed) -> None:
        self._schedule_preview()

    def _schedule_preview(self) -> None:
        """Debounce preview updates to avoid jank on rapid input."""
        if self._preview_timer is not None:
            self._preview_timer.stop()
        self._preview_timer = self.set_timer(PREVIEW_DEBOUNCE_SEC, self._update_preview)

    def _validate_field(self, widget: Input) -> None:
        """Inline validation — add/remove 'error' CSS class on the input."""
        wid = widget.id or ""
        val = widget.value.strip()
        error = False
        if wid == "input-time" and val:
            try:
                parse_time(val)
            except ValueError:
                error = True
        elif wid == "input-memory" and val:
            try:
                parse_memory(val)
            except ValueError:
                error = True
        elif wid == "input-gpus" and val:
            if not _valid_gpu_spec(val):
                error = True
        elif wid == "input-name" and val:
            try:
                validate_job_name(val)
            except ValueError:
                error = True

        widget.set_class(error, "-invalid")

    # ---- Form value helpers -----------------------------------------------

    def _val(self, widget_id: str) -> str:
        try:
            return self.query_one(f"#{widget_id}", Input).value.strip()
        except LookupError:
            return ""

    def _sel(self, widget_id: str) -> str:
        try:
            v = self.query_one(f"#{widget_id}", Select).value
            if v is None or v is Select.BLANK:
                return ""
            return str(v)
        except LookupError:
            return ""

    def _txt(self, widget_id: str) -> str:
        try:
            return self.query_one(f"#{widget_id}", TextArea).text.strip()
        except LookupError:
            return ""

    # ---- Click handling ---------------------------------------------------

    def on_click(self, event: Click) -> None:
        """Handle clicks on Static help/remove links."""
        widget = event.widget
        if widget is None:
            return
        wid = widget.id or ""

        # Help links: "help-<param_key>"
        if wid.startswith("help-"):
            self._show_help(wid[5:])
            return

        # Extra param help: "xh-<uid>-<key>"
        if wid.startswith("xh-"):
            parts = wid.split("-", 2)
            if len(parts) == 3:
                self._show_help(parts[2])
            return

        # Extra param remove: "xrm-<uid>-<key>"
        if wid.startswith("xrm-"):
            parts = wid.split("-", 2)
            if len(parts) == 3:
                self._remove_extra(parts[2])
            return

    def on_button_pressed(self, event: Button.Pressed) -> None:
        bid = event.button.id or ""
        if bid == "btn-add-param":
            self._open_add_param()
        elif bid == "btn-submit":
            self.action_submit()
        elif bid == "btn-save-tmpl":
            self.action_save_template()
        elif bid == "btn-load-tmpl":
            self.action_load_template()
        elif bid == "btn-import-sbatch":
            self.action_import_sbatch()

    def _show_help(self, param_key: str) -> None:
        from slurm_term.screens.add_param_screen import ParamHelpScreen
        self.app.push_screen(ParamHelpScreen(param_key))

    # ---- Dynamic extra params ---------------------------------------------

    def _open_add_param(self) -> None:
        from slurm_term.screens.add_param_screen import AddParamScreen
        existing = {k for k, _, _, _ in self._extra_params}
        self.app.push_screen(
            AddParamScreen(exclude_keys=existing),
            callback=self._on_param_added,
        )

    def _on_param_added(self, result: tuple[str, str, str, str] | None) -> None:
        if result is None:
            return
        key = result[0]
        if any(k == key for k, _, _, _ in self._extra_params):
            return
        self._extra_params.append(result)
        self._render_extras()
        self._update_preview()

    def _remove_extra(self, key: str) -> None:
        self._extra_params = [p for p in self._extra_params if p[0] != key]
        self._render_extras()
        self._update_preview()

    def _render_extras(self) -> None:
        container = self.query_one("#extras-container", Vertical)
        container.remove_children()
        self._uid_to_key.clear()
        for key, label, short_desc, _ in self._extra_params:
            is_flag = key in FLAG_PARAMS
            uid = next(self._uid_counter)
            self._uid_to_key[uid] = key

            # Header line: --key  [?] [x]
            hdr = Horizontal(classes="extra-header", id=f"eh-{uid}")
            container.mount(hdr)
            hdr.mount(Static(f"--{key}", classes="extra-label"))
            hdr.mount(Static("[?]", classes="extra-link", id=f"xh-{uid}-{key}"))
            hdr.mount(Static("[x]", classes="extra-rm", id=f"xrm-{uid}-{key}"))

            # Input or flag note
            if is_flag:
                container.mount(Static(
                    f"  (flag, no value needed)",
                    classes="extra-flag-note",
                ))
            else:
                container.mount(Input(
                    placeholder=short_desc,
                    id=f"ext-{uid}",
                    classes="extra-input",
                ))

    def _get_extra_val(self, key: str) -> str:
        if key in FLAG_PARAMS:
            return ""
        for uid, k in self._uid_to_key.items():
            if k == key:
                try:
                    return self.query_one(f"#ext-{uid}", Input).value.strip()
                except LookupError:
                    pass
        return ""

    # ---- Templates --------------------------------------------------------

    def _get_form_state(self) -> dict[str, str]:
        """Capture the current form state as a serializable dict."""
        state: dict[str, str] = {
            "mode": self._sel("select-mode"),
            "partition": self._sel("select-partition"),
            "time": self._val("input-time"),
            "nodes": self._val("input-nodes"),
            "ntasks": self._val("input-ntasks"),
            "cpus": self._val("input-cpus"),
            "memory": self._val("input-memory"),
            "gpus": self._val("input-gpus"),
            "name": self._val("input-name"),
            "script": self._val("input-script"),
            "output": self._val("input-output"),
            "error": self._val("input-error"),
            "modules": self._txt("input-modules"),
            "env": self._txt("input-env"),
            "init": self._txt("input-init"),
        }
        return state

    def set_form_state(self, state: dict[str, str]) -> None:
        """Restore form state from a saved dict."""
        # Coerce all values to strings for safety (handles corrupted templates)
        state = {k: str(v) if v is not None else "" for k, v in state.items()}
        field_map = {
            "time": "input-time", "nodes": "input-nodes",
            "ntasks": "input-ntasks", "cpus": "input-cpus",
            "memory": "input-memory", "gpus": "input-gpus",
            "name": "input-name", "script": "input-script",
            "output": "input-output", "error": "input-error",
        }
        for key, wid in field_map.items():
            if key in state:
                try:
                    self.query_one(f"#{wid}", Input).value = state[key]
                except LookupError:
                    pass

        textarea_map = {
            "modules": "input-modules", "env": "input-env", "init": "input-init",
        }
        for key, wid in textarea_map.items():
            if key in state:
                try:
                    self.query_one(f"#{wid}", TextArea).load_text(state[key])
                except LookupError:
                    pass

        if "mode" in state:
            try:
                sel = self.query_one("#select-mode", Select)
                if state["mode"]:
                    sel.value = state["mode"]
                else:
                    sel.clear()
                self._update_mode_visibility()
            except (LookupError, Exception):
                pass

        if "partition" in state:
            try:
                sel = self.query_one("#select-partition", Select)
                if state["partition"]:
                    sel.value = state["partition"]
                else:
                    sel.clear()
            except (LookupError, Exception):
                pass

        self._update_preview()

    def action_save_template(self) -> None:
        from slurm_term.screens.templates import SaveTemplateScreen
        self.app.push_screen(SaveTemplateScreen(), callback=self._on_save_template)

    def _on_save_template(self, name: str | None) -> None:
        if not name:
            return
        from slurm_term.screens.templates import save_template
        save_template(name, self._get_form_state())
        self.set_status(f"Template '{name}' saved")

    def action_load_template(self) -> None:
        from slurm_term.screens.templates import LoadTemplateScreen
        self.app.push_screen(LoadTemplateScreen(), callback=self._on_load_template)

    def _on_load_template(self, name: str | None) -> None:
        if not name:
            return
        from slurm_term.screens.templates import load_template
        data = load_template(name)
        if data and isinstance(data, dict):
            self.set_form_state(data)
            self.set_status(f"Template '{name}' loaded")
        else:
            self.set_status(f"Template '{name}' not found or invalid")

    # ---- Import sbatch file -----------------------------------------------

    def action_import_sbatch(self) -> None:
        from slurm_term.screens.import_sbatch_screen import ImportSbatchScreen
        self.app.push_screen(ImportSbatchScreen(), callback=self._on_import_sbatch)

    def _on_import_sbatch(self, path: str | None) -> None:
        if not path:
            return
        try:
            from slurm_term.sbatch_parser import parse_sbatch_file
            data = parse_sbatch_file(path)
        except (FileNotFoundError, ValueError, OSError) as e:
            self.set_status(f"! Import failed: {e}")
            return
        except Exception as e:
            self.set_status(f"! Import failed unexpectedly: {e}")
            return

        # Apply the parsed state
        self.set_form_state(data)

        # Handle extra directives as additional params
        extra = data.get("extra_directives", {})
        if extra and isinstance(extra, dict):
            for key, value in extra.items():
                if not any(k == key for k, _, _, _ in self._extra_params):
                    self._extra_params.append(
                        (key, f"--{key}", value or "(imported)", ""),
                    )
            self._render_extras()
            # Set values for extras
            for key, value in extra.items():
                if value:
                    for uid, k in self._uid_to_key.items():
                        if k == key:
                            try:
                                self.query_one(f"#ext-{uid}", Input).value = value
                            except LookupError:
                                pass

        import os.path
        basename = os.path.basename(path)
        self.set_status(f"Imported '{basename}' — review and submit")

    # ---- Build params dict ------------------------------------------------

    def _build_params(self) -> dict[str, str]:
        params: dict[str, str] = {}
        if v := self._sel("select-partition"):
            params["partition"] = v
        if v := self._val("input-time"):
            params["time"] = v
        if v := self._val("input-nodes"):
            params["nodes"] = v
        if v := self._val("input-ntasks"):
            params["ntasks-per-node"] = v
        if v := self._val("input-cpus"):
            params["cpus-per-task"] = v
        if v := self._val("input-memory"):
            params["mem"] = v
        if v := self._val("input-gpus"):
            params["gres"] = f"gpu:{v}"

        if not self._is_srun():
            if v := self._val("input-name"):
                params["job-name"] = v
            if v := self._val("input-output"):
                params["output"] = v
            if v := self._val("input-error"):
                params["error"] = v

        for key, _, _, _ in self._extra_params:
            val = self._get_extra_val(key)
            params[key] = val

        return params

    # ---- Preview ----------------------------------------------------------

    def _update_preview(self) -> None:
        try:
            preview = self.query_one("#composer-preview", RichLog)
        except LookupError:
            return
        preview.clear()
        if self._is_srun():
            self._preview_srun(preview)
        else:
            self._preview_sbatch(preview)

    def _preview_sbatch(self, preview: RichLog) -> None:
        lines = ["#!/bin/bash"]
        params = self._build_params()
        for k, v in params.items():
            if v:
                lines.append(f"#SBATCH --{k}={v}")
            else:
                lines.append(f"#SBATCH --{k}")

        modules = self._txt("input-modules")
        env_vars = self._txt("input-env")
        init_cmds = self._txt("input-init")
        if modules or env_vars or init_cmds:
            lines.append("")
        if modules:
            for mod in modules.splitlines():
                mod = mod.strip()
                if mod:
                    lines.append(f"module load {mod}")
        if env_vars:
            lines.append("")
            for var in env_vars.splitlines():
                var = var.strip()
                if var:
                    lines.append(f"export {var}")
        if init_cmds:
            lines.append("")
            for cmd in init_cmds.splitlines():
                lines.append(cmd)

        script = self._val("input-script")
        if script:
            lines.append("")
            lines.append(script)
        preview.write("\n".join(lines))
        self._last_preview_text = "\n".join(lines)

    def _preview_srun(self, preview: RichLog) -> None:
        params = self._build_params()
        parts = ["srun", "--pty"]
        for k, v in params.items():
            if v:
                parts.append(f"--{k}={v}")
            else:
                parts.append(f"--{k}")
        shell = os.environ.get("SHELL", "/bin/bash")
        parts.append(shell)
        preview.write(" \\\n  ".join(parts))
        self._last_preview_text = " \\\n  ".join(parts)

    def action_copy_preview(self) -> None:
        """Copy the generated script to the system clipboard."""
        if self._last_preview_text:
            self.app.copy_to_clipboard(self._last_preview_text)
            self.set_status("Script copied to clipboard")
        else:
            self.set_status("Nothing to copy")

    # ---- Validation -------------------------------------------------------

    def set_status(self, msg: str) -> None:
        self.query_one("#composer-status", Static).update(f" {msg}")

    def _validate(self) -> str | None:
        if not self._is_srun():
            try:
                validate_job_name(self._val("input-name"))
            except ValueError as e:
                return str(e)
            script = self._val("input-script")
            init = self._txt("input-init")
            # Either a script path or inline init commands are required
            if not script and not init:
                return "Provide a script path or init commands"
            if script and not os.path.isfile(script):
                return f"Script not found: {script}"

        t = self._val("input-time")
        if t:
            try:
                parse_time(t)
            except ValueError:
                return f"Invalid time format: {t!r}"

        mem = self._val("input-memory")
        if mem:
            try:
                parse_memory(mem)
            except ValueError:
                return f"Invalid memory format: {mem!r}"

        gpus = self._val("input-gpus")
        if gpus and not _valid_gpu_spec(gpus):
            return f"Invalid GPU spec: {gpus!r} (expected e.g. '1' or 'a100:2')"

        if not self._sel("select-partition"):
            return "Select a partition"
        return None

    # ---- Submission / Launch ----------------------------------------------

    def action_submit(self) -> None:
        error = self._validate()
        if error:
            self.set_status(f"! {error}")
            return

        from slurm_term.screens.confirm import ConfirmScreen

        if self._is_srun():
            part = self._sel("select-partition")
            t = self._val("input-time")
            self.app.push_screen(
                ConfirmScreen(f"Launch srun on [b]{escape_markup(part)}[/b] for [b]{escape_markup(t)}[/b]?"),
                callback=self._on_confirm_srun,
            )
        else:
            name = self._val("input-name")
            self.app.push_screen(
                ConfirmScreen(f"Submit batch job [b]{escape_markup(name)}[/b]?"),
                callback=self._on_confirm_sbatch,
            )

    def _on_confirm_sbatch(self, confirmed: bool) -> None:
        if not confirmed:
            return
        params = self._build_params()
        script = self._val("input-script")
        init = self._txt("input-init")
        modules = self._txt("input-modules")
        env_vars = self._txt("input-env")
        self.set_status("Submitting job...")
        self.run_worker(
            lambda: self._run_submit(script, params, init, modules, env_vars),
            group="job-submit", exclusive=True,
        )

    async def _run_submit(
        self,
        script: str,
        params: dict[str, str],
        init: str = "",
        modules: str = "",
        env_vars: str = "",
    ) -> None:
        loop = asyncio.get_running_loop()
        try:
            if script:
                # Standard file-based submission
                job_id = await loop.run_in_executor(
                    None, self.slurm.submit_job, script, params,
                )
            else:
                # Wrap-mode: build inline command string
                wrap_lines: list[str] = []
                if modules:
                    for mod in modules.splitlines():
                        mod = mod.strip()
                        if mod:
                            wrap_lines.append(f"module load {mod}")
                if env_vars:
                    for var in env_vars.splitlines():
                        var = var.strip()
                        if var:
                            wrap_lines.append(f"export {var}")
                if init:
                    for cmd in init.splitlines():
                        wrap_lines.append(cmd)
                commands = "; ".join(wrap_lines)
                job_id = await loop.run_in_executor(
                    None, self.slurm.submit_wrap, commands, params,
                )
            self.set_status(f"✓ Job {job_id} submitted")
            self.app.notify(
                f"Job {job_id} submitted successfully",
                title="Job Submitted",
                severity="information",
                timeout=6,
            )
            self.post_message(self.JobSubmitted(job_id))
        except (RuntimeError, ValueError, OSError) as e:
            self.set_status(f"! {e}")
        except Exception as e:
            self.set_status(f"! Unexpected error: {e}")

    def _on_confirm_srun(self, confirmed: bool) -> None:
        if not confirmed:
            return
        params = self._build_params()
        params["pty"] = ""
        shell = os.environ.get("SHELL", "/bin/bash")

        self.set_status("Launching srun...")
        try:
            with self.app.suspend():
                print("\n--- SlurmTerm: Interactive srun Session ---")
                print("    Type 'exit' to return to SlurmTerm.\n")
                rc, stderr_text = self.slurm.srun([shell], params=params)
                print(f"\nsrun exited with code {rc}")
                if rc != 0:
                    input("Press Enter to return to SlurmTerm...")
                else:
                    print("Returning to SlurmTerm...\n")
        except (ValueError, OSError) as e:
            self.set_status(f"! srun launch failed: {e}")
            return
        except Exception as e:
            self.set_status(f"! srun failed unexpectedly: {e}")
            return

        if rc != 0 and stderr_text:
            # Truncate for the notification toast (keep last few lines).
            lines = stderr_text.strip().splitlines()
            summary = "\n".join(lines[-5:]) if len(lines) > 5 else "\n".join(lines)
            self.app.notify(
                summary, title="srun failed", severity="error", timeout=12
            )
        if rc == 0:
            self.set_status("✓ Interactive session ended successfully")
            self.app.notify(
                "Interactive session completed",
                title="srun Finished",
                severity="information",
                timeout=6,
            )
        else:
            self.set_status(f"✗ srun failed (exit code {rc})")


def _valid_gpu_spec(spec: str) -> bool:
    """Check that a GPU spec looks like '1', '2', 'a100:2', etc."""
    return bool(_GPU_SPEC_RE.match(spec.strip()))
