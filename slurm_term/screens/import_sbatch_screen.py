"""Modal screen for importing an .sbatch file into the Composer."""

from __future__ import annotations

from pathlib import Path

from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Vertical, Horizontal
from textual.screen import ModalScreen
from textual.widgets import Static, Input, Button


class ImportSbatchScreen(ModalScreen[str | None]):
    """Modal that prompts for a .sbatch file path and returns it on success."""

    BINDINGS = [
        Binding("escape", "cancel", "Cancel", show=False),
    ]

    DEFAULT_CSS = """
    ImportSbatchScreen {
        align: center middle;
    }
    #import-dialog {
        width: 70;
        height: auto;
        border: thick $accent;
        background: $surface;
        padding: 1 2;
    }
    #import-title {
        text-style: bold;
        margin-bottom: 1;
    }
    #import-hint {
        color: $text-muted;
        text-style: italic;
        margin-bottom: 1;
    }
    #import-error {
        color: $error;
        text-style: bold;
        height: auto;
        margin-bottom: 1;
    }
    #import-input {
        margin-bottom: 1;
    }
    #import-buttons {
        height: 3;
        align: center middle;
    }
    #import-buttons Button {
        margin: 0 1;
        min-width: 12;
    }
    """

    def compose(self) -> ComposeResult:
        with Vertical(id="import-dialog"):
            yield Static(
                "[b]Import .sbatch File[/b]", id="import-title", markup=True,
            )
            yield Static(
                "Enter the full path to a .sbatch script file.",
                id="import-hint",
            )
            yield Static("", id="import-error")
            yield Input(
                placeholder="/path/to/job.sbatch",
                id="import-input",
            )
            with Horizontal(id="import-buttons"):
                yield Button(
                    "Import", variant="success", id="btn-import-ok",
                )
                yield Button(
                    "Cancel", variant="default", id="btn-import-cancel",
                )

    def on_mount(self) -> None:
        self.query_one("#import-input", Input).focus()

    def on_input_submitted(self, event: Input.Submitted) -> None:
        """Allow Enter to trigger import."""
        if event.input.id == "import-input":
            self._try_import()

    def on_button_pressed(self, event: Button.Pressed) -> None:
        bid = event.button.id or ""
        if bid == "btn-import-ok":
            self._try_import()
        elif bid == "btn-import-cancel":
            self.dismiss(None)

    def _try_import(self) -> None:
        raw = self.query_one("#import-input", Input).value.strip()
        error_widget = self.query_one("#import-error", Static)

        if not raw:
            error_widget.update("[b]Please enter a file path[/b]")
            return

        # Expand ~ and resolve
        path = Path(raw).expanduser().resolve()

        if not path.is_file():
            error_widget.update(f"[b]File not found:[/b] {path}")
            return

        if path.suffix not in (".sbatch", ".sh", ".bash", ".slurm"):
            error_widget.update(
                f"[b]Unexpected extension:[/b] {path.suffix}  "
                "(expected .sbatch, .sh, .bash, or .slurm)"
            )
            return

        self.dismiss(str(path))

    def action_cancel(self) -> None:
        self.dismiss(None)
