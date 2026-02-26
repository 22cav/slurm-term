"""Reusable confirmation modal screen."""

from __future__ import annotations

from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Horizontal, Vertical
from textual.screen import ModalScreen
from textual.widgets import Static, Button


class ConfirmScreen(ModalScreen[bool]):
    """A yes/no confirmation dialog.

    Returns ``True`` if confirmed, ``False`` if cancelled.
    """

    BINDINGS = [
        Binding("escape", "cancel", "Cancel", show=False),
    ]

    DEFAULT_CSS = """
    ConfirmScreen {
        align: center middle;
    }

    #confirm-dialog {
        width: 50;
        height: auto;
        max-height: 12;
        border: thick $accent;
        background: $surface;
        padding: 1 2;
    }

    #confirm-message {
        width: 100%;
        content-align: center middle;
        margin-bottom: 1;
    }

    #confirm-buttons {
        width: 100%;
        height: 3;
        align: center middle;
    }

    #confirm-buttons Button {
        margin: 0 1;
        min-width: 10;
    }
    """

    def __init__(self, message: str, **kwargs) -> None:
        super().__init__(**kwargs)
        self._message = message

    def compose(self) -> ComposeResult:
        with Vertical(id="confirm-dialog"):
            yield Static(self._message, id="confirm-message")
            with Horizontal(id="confirm-buttons"):
                yield Button("Yes", variant="warning", id="confirm-yes")
                yield Button("No", variant="default", id="confirm-no")

    def on_button_pressed(self, event: Button.Pressed) -> None:
        self.dismiss(event.button.id == "confirm-yes")

    def action_cancel(self) -> None:
        self.dismiss(False)
