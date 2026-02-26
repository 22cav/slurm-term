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
from textual.timer import Timer
from textual.widgets import Static, Input, Select, Button, TextArea, RichLog, Label

from slurm_term.slurm_api import SlurmController
from slurm_term.utils.formatting import escape_markup
from slurm_term.utils.validators import parse_time, parse_memory, validate_job_name
from slurm_term.screens.param_catalog import FLAG_PARAMS

_GPU_SPEC_RE = re.compile(r"^[a-zA-Z0-9_]*:?\d+$")


class ComposerTab(Vertical):
    """Unified job submission form (sbatch + srun) with live preview."""

    BINDINGS = [
        Binding("ctrl+s", "submit", "Submit / Launch", show=True),
        Binding("ctrl+t", "save_template", "Save Template", show=True),
        Binding("ctrl+l", "load_template", "Load Template", show=True),

    ]

    DEFAULT_CSS = """
    ComposerTab {
        height: 1fr;
    }
    #composer-grid {
        height: 1fr;
        layout: grid;
        grid-size: 2 1;
        grid-columns: 1fr 1fr;
        grid-gutter: 1;
        padding: 1;
    }
    #composer-form {
        height: 1fr;
        padding: 0 1;
    }
    #composer-preview-pane {
        height: 1fr;
    }
    #composer-preview {
        height: 1fr;
        border: solid $accent;
    }

    /* Label row: text + clickable [?] on one line */
    .form-label-row {
        height: 1;
        margin-top: 1;
    }
    .form-label-text {
        text-style: bold;
        width: 1fr;
    }
    .help-link {
        width: auto;
        color: $accent;
        text-style: bold;
    }

    .form-section {
        margin-top: 1;
        text-style: bold italic;
        color: $accent;
    }
    #btn-submit, #btn-add-param, #btn-save-tmpl, #btn-load-tmpl {
        margin-top: 1;
        width: 100%;
    }
    #extras-container {
        height: auto;
    }

    /* Extra param row: --key [?] [x]  then input below */
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
        color: $accent;
        text-style: bold;
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

    #input-modules, #input-env {
        height: 4;
    }
    #input-init {
        height: 6;
    }
    Input.-invalid {
        border: tall $error;
    }
    """

    def __init__(self, slurm: SlurmController | None = None, **kwargs) -> None:
        super().__init__(**kwargs)
        self.slurm = slurm or SlurmController()
        self._extra_params: list[tuple[str, str, str, str]] = []
        self._uid_to_key: dict[int, str] = {}
        self._uid_counter = count()
        self._preview_timer: Timer | None = None

    def compose(self) -> ComposeResult:
        with Horizontal(id="composer-grid"):
            with VerticalScroll(id="composer-form"):
                # ---- Mode ----
                yield Label("Mode", classes="form-section")
                yield Select(
                    [("sbatch (Batch Job)", "sbatch"), ("srun (Interactive)", "srun")],
                    value="sbatch", id="select-mode",
                )

                # ---- Core resource fields ----
                yield Label("── Resources ──", classes="form-section")

                yield self._label_row("Partition", "partition")
                yield Select([], id="select-partition", prompt="Select partition")

                yield self._label_row("Time Limit (HH:MM:SS)", "time")
                yield Input(value="01:00:00", id="input-time")

                yield self._label_row("Nodes", "nodes")
                yield Input(value="1", id="input-nodes")

                yield self._label_row("Tasks per Node", "ntasks-per-node")
                yield Input(value="1", id="input-ntasks")

                yield self._label_row("CPUs per Task", "cpus-per-task")
                yield Input(value="1", id="input-cpus")

                yield self._label_row("Memory (e.g. 4G)", "mem")
                yield Input(value="4G", id="input-memory")

                yield self._label_row("GPUs (e.g. 1 or a100:2)", "gres")
                yield Input(id="input-gpus")

                # ---- sbatch-only fields ----
                yield Label("── Batch Job ──", classes="form-section", id="section-batch")

                yield self._label_row("Job Name", "job-name", lbl_id="lbl-name")
                yield Input(value="my_job", id="input-name")

                yield self._label_row("Script Path", "_script-path", lbl_id="lbl-script")
                yield Input(placeholder="/path/to/script.sh", id="input-script")

                yield self._label_row("Output Pattern", "output", lbl_id="lbl-output")
                yield Input(value="%x-%j.out", id="input-output")

                yield self._label_row("Error Pattern", "error", lbl_id="lbl-error")
                yield Input(value="%x-%j.err", id="input-error")

                # ---- Environment (sbatch only) ----
                yield Label("── Environment ──", classes="form-section", id="section-env")

                yield self._label_row("Module Loads (one per line)", "_modules", lbl_id="lbl-modules")
                yield TextArea(id="input-modules", language=None, soft_wrap=True)

                yield self._label_row("Env Vars (KEY=VALUE)", "_env-vars", lbl_id="lbl-env")
                yield TextArea(id="input-env", language=None, soft_wrap=True)

                yield self._label_row("Init Commands (shell)", "_init-cmds", lbl_id="lbl-init")
                yield TextArea(id="input-init", language="bash", soft_wrap=True)

                # ---- Dynamic extras ----
                yield Label("── Additional Parameters ──", classes="form-section")
                yield Vertical(id="extras-container")
                yield Button("+ Add Parameter...", variant="primary", id="btn-add-param")

                # ---- Templates ----
                yield Label("── Templates ──", classes="form-section")
                yield Button("Save as Template (Ctrl+T)", variant="primary", id="btn-save-tmpl")
                yield Button("Load Template (Ctrl+L)", variant="default", id="btn-load-tmpl")

                # ---- Submit ----
                yield Button("Submit Job", variant="success", id="btn-submit")

            with Vertical(id="composer-preview-pane"):
                yield Label("Preview", classes="form-section")
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
        row.compose_add_child(Static("[?]", classes="help-link", id=f"help-{param_key}"))
        return row

    def on_mount(self) -> None:
        self._update_mode_visibility()
        self._update_preview()
        self.run_worker(self._fetch_partitions, group="partitions")

    async def _fetch_partitions(self) -> None:
        loop = asyncio.get_running_loop()
        parts = await loop.run_in_executor(None, self.slurm.get_partitions)
        if parts:
            sel = self.query_one("#select-partition", Select)
            sel.set_options([(p, p) for p in parts])
            sel.value = parts[0]

    # ---- Mode toggle ------------------------------------------------------

    def _is_srun(self) -> bool:
        return self._sel("select-mode") == "srun"

    def _update_mode_visibility(self) -> None:
        srun = self._is_srun()
        sbatch_ids = [
            "section-batch", "lbl-name", "input-name",
            "lbl-script", "input-script",
            "lbl-output", "input-output",
            "lbl-error", "input-error",
            "section-env", "lbl-modules", "input-modules",
            "lbl-env", "input-env", "lbl-init", "input-init",
        ]
        for wid in sbatch_ids:
            try:
                self.query_one(f"#{wid}").display = not srun
            except LookupError:
                pass
        try:
            btn = self.query_one("#btn-submit", Button)
            btn.label = "Launch Interactive Session" if srun else "Submit Job"
        except LookupError:
            pass

    # ---- Event handlers ---------------------------------------------------

    def on_input_changed(self, event: Input.Changed) -> None:
        self._schedule_preview()
        self._validate_field(event.input)

    def on_select_changed(self, event: Select.Changed) -> None:
        if event.select.id == "select-mode":
            self._update_mode_visibility()
        self._schedule_preview()

    def on_text_area_changed(self, event: TextArea.Changed) -> None:
        self._schedule_preview()

    def _schedule_preview(self) -> None:
        """Debounce preview updates to avoid jank on rapid input."""
        if self._preview_timer is not None:
            self._preview_timer.stop()
        self._preview_timer = self.set_timer(0.3, self._update_preview)

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

    def _set_form_state(self, state: dict[str, str]) -> None:
        """Restore form state from a saved dict."""
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
                self.query_one("#select-mode", Select).value = state["mode"]
                self._update_mode_visibility()
            except LookupError:
                pass

        if "partition" in state:
            try:
                self.query_one("#select-partition", Select).value = state["partition"]
            except LookupError:
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
        self._set_status(f"Template '{name}' saved")

    def action_load_template(self) -> None:
        from slurm_term.screens.templates import LoadTemplateScreen
        self.app.push_screen(LoadTemplateScreen(), callback=self._on_load_template)

    def _on_load_template(self, name: str | None) -> None:
        if not name:
            return
        from slurm_term.screens.templates import load_template
        data = load_template(name)
        if data:
            self._set_form_state(data)
            self._set_status(f"Template '{name}' loaded")
        else:
            self._set_status(f"Template '{name}' not found")

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

    # ---- Validation -------------------------------------------------------

    def _set_status(self, msg: str) -> None:
        self.query_one("#composer-status", Static).update(f" {msg}")

    def _validate(self) -> str | None:
        if not self._is_srun():
            try:
                validate_job_name(self._val("input-name"))
            except ValueError as e:
                return str(e)
            script = self._val("input-script")
            if not script:
                return "Script path is required"
            if not os.path.isfile(script):
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
            self._set_status(f"! {error}")
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
        self._set_status("Submitting job...")
        self.run_worker(
            lambda: self._run_submit(script, params),
            group="job-submit", exclusive=True,
        )

    async def _run_submit(self, script: str, params: dict[str, str]) -> None:
        loop = asyncio.get_running_loop()
        try:
            job_id = await loop.run_in_executor(
                None, self.slurm.submit_job, script, params,
            )
            self._set_status(f"Submitted job {job_id}")
        except (RuntimeError, ValueError) as e:
            self._set_status(f"! {e}")

    def _on_confirm_srun(self, confirmed: bool) -> None:
        if not confirmed:
            return
        params = self._build_params()
        params["pty"] = ""
        shell = os.environ.get("SHELL", "/bin/bash")

        self._set_status("Launching srun...")
        with self.app.suspend():
            print("\n--- SlurmTerm: Interactive srun Session ---")
            print("    Type 'exit' to return to SlurmTerm.\n")
            rc = self.slurm.srun([shell], params=params)
            print(f"\nsrun exited with code {rc}")
            print("Returning to SlurmTerm...\n")
        self._set_status(f"srun session ended (exit code {rc})")


def _valid_gpu_spec(spec: str) -> bool:
    """Check that a GPU spec looks like '1', '2', 'a100:2', etc."""
    return bool(_GPU_SPEC_RE.match(spec.strip()))
