"""Modal screen for adding optional Slurm parameters, with info panel."""

from __future__ import annotations

from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Vertical, Horizontal, VerticalScroll
from textual.screen import ModalScreen
from textual.widgets import Static, Input, Button

from slurm_term.screens.param_catalog import EXTRA_PARAMS, PARAM_BY_KEY


class AddParamScreen(ModalScreen[tuple[str, str, str, str] | None]):
    """Searchable catalog of Slurm parameters with detail panel.

    Returns ``(key, label, short_desc, long_desc)`` on selection,
    or ``None`` on cancel.
    """

    BINDINGS = [
        Binding("escape", "cancel", "Cancel", show=False),
    ]

    DEFAULT_CSS = """
    AddParamScreen {
        align: center middle;
    }
    #add-param-dialog {
        width: 90;
        height: 85%;
        border: thick $accent;
        background: $surface;
        padding: 1 2;
    }
    #param-search {
        margin-bottom: 1;
    }
    #param-body {
        height: 1fr;
        layout: grid;
        grid-size: 2 1;
        grid-columns: 1fr 1fr;
        grid-gutter: 1;
    }
    #param-list-scroll {
        height: 1fr;
    }
    #param-detail-pane {
        height: 1fr;
        overflow-y: auto;
        border: solid $accent;
        padding: 1 2;
        background: $surface-darken-1;
    }
    .param-btn {
        width: 100%;
        height: auto;
        min-height: 1;
        margin-bottom: 0;
        content-align: left middle;
    }
    .param-btn:hover {
        background: $accent 30%;
    }
    #btn-cancel-params {
        margin-top: 1;
        width: 100%;
    }
    #detail-title {
        text-style: bold;
        color: $accent;
        margin-bottom: 1;
    }
    #detail-body {
        color: $text;
    }
    """

    def __init__(self, exclude_keys: set[str] | None = None, **kwargs) -> None:
        super().__init__(**kwargs)
        self._exclude = exclude_keys or set()
        self._selected_key: str | None = None

    def compose(self) -> ComposeResult:
        with Vertical(id="add-param-dialog"):
            yield Static(
                "[b]Add Parameter[/b]  —  click a parameter to see details, "
                "then press [b]Add[/b] to add it",
                markup=True,
            )
            yield Input(placeholder="Search parameters…", id="param-search")
            with Horizontal(id="param-body"):
                with VerticalScroll(id="param-list-scroll"):
                    for key, label, short_desc, _ in EXTRA_PARAMS:
                        if key not in self._exclude:
                            yield Button(
                                f"--{key}",
                                id=f"param-{key}",
                                variant="default",
                                classes="param-btn",
                            )
                with Vertical(id="param-detail-pane"):
                    yield Static("Select a parameter to view details", id="detail-title")
                    yield Static("", id="detail-body")
                    yield Button("Add Selected", variant="success", id="btn-add-selected", disabled=True)
            yield Button("Cancel", variant="default", id="btn-cancel-params")

    def on_input_changed(self, event: Input.Changed) -> None:
        query = event.value.strip().lower()
        for key, label, short_desc, long_desc in EXTRA_PARAMS:
            if key in self._exclude:
                continue
            try:
                btn = self.query_one(f"#param-{key}", Button)
                match = (
                    not query
                    or query in key.lower()
                    or query in label.lower()
                    or query in short_desc.lower()
                    or query in long_desc.lower()
                )
                btn.display = match
            except LookupError:
                pass

    def on_button_pressed(self, event: Button.Pressed) -> None:
        bid = event.button.id or ""

        if bid == "btn-cancel-params":
            self.dismiss(None)
            return

        if bid == "btn-add-selected" and self._selected_key:
            entry = PARAM_BY_KEY.get(self._selected_key)
            if entry:
                self.dismiss(entry)
            return

        if bid.startswith("param-"):
            key = bid[6:]
            self._selected_key = key
            entry = PARAM_BY_KEY.get(key)
            if entry:
                _, label, short_desc, long_desc = entry
                self.query_one("#detail-title", Static).update(
                    f"[bold]--{key}[/bold]  ({label})"
                )
                self.query_one("#detail-body", Static).update(long_desc)
                self.query_one("#btn-add-selected", Button).disabled = False

    def action_cancel(self) -> None:
        self.dismiss(None)


class ParamHelpScreen(ModalScreen[None]):
    """Read-only help screen for viewing a parameter's documentation."""

    BINDINGS = [
        Binding("escape", "close", "Close", show=False),
    ]

    DEFAULT_CSS = """
    ParamHelpScreen {
        align: center middle;
    }
    #help-dialog {
        width: 70;
        height: 60%;
        border: thick $accent;
        background: $surface;
        padding: 1 2;
    }
    #help-title {
        text-style: bold;
        color: $accent;
        margin-bottom: 1;
    }
    #help-body {
        height: 1fr;
        overflow-y: auto;
    }
    #btn-close-help {
        margin-top: 1;
        width: 100%;
    }
    """

    def __init__(self, key: str, **kwargs) -> None:
        super().__init__(**kwargs)
        self._key = key

    def compose(self) -> ComposeResult:
        entry = PARAM_BY_KEY.get(self._key)
        if entry:
            _, label, _, long_desc = entry
            title = f"--{self._key}  ({label})"
            body = long_desc
        else:
            title = f"--{self._key}"
            body = "No documentation available."

        with Vertical(id="help-dialog"):
            yield Static(f"[bold]{title}[/bold]", markup=True, id="help-title")
            yield Static(body, id="help-body")
            yield Button("Close", variant="primary", id="btn-close-help")

    def on_button_pressed(self, event: Button.Pressed) -> None:
        if event.button.id == "btn-close-help":
            self.dismiss(None)

    def action_close(self) -> None:
        self.dismiss(None)
