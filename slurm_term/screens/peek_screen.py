"""Quick-peek modal — shows last lines of a job's output file."""

from __future__ import annotations

from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Vertical
from textual.screen import ModalScreen
from textual.widgets import RichLog, Static


class PeekScreen(ModalScreen[None]):
    """Read-only modal displaying the tail of a log file."""

    BINDINGS = [Binding("escape", "dismiss", "Close", show=True)]

    DEFAULT_CSS = """
    PeekScreen {
        align: center middle;
    }
    #peek-dialog {
        width: 90%;
        height: 80%;
        border: thick $accent;
        background: $surface;
    }
    #peek-title {
        dock: top;
        padding: 0 2;
        background: $primary-background;
        text-style: bold;
    }
    #peek-log {
        height: 1fr;
        padding: 0 1;
    }
    """

    def __init__(self, title: str, content: str, **kwargs) -> None:
        super().__init__(**kwargs)
        self._title = title
        self._content = content

    def compose(self) -> ComposeResult:
        with Vertical(id="peek-dialog"):
            yield Static(self._title, id="peek-title")
            yield RichLog(id="peek-log", wrap=True, highlight=True)

    def on_mount(self) -> None:
        self.query_one("#peek-log", RichLog).write(self._content)
