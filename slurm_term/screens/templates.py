"""Job template save/load — persist Composer state as JSON."""

from __future__ import annotations

import json
import os
import re
from pathlib import Path
from typing import Any

from textual.app import ComposeResult
from textual.binding import Binding
from textual.containers import Vertical, Horizontal, VerticalScroll
from textual.screen import ModalScreen
from textual.widgets import Static, Input, Button

TEMPLATES_DIR = Path(os.environ.get(
    "SLURMTERM_TEMPLATES_DIR",
    Path.home() / ".config" / "slurmterm" / "templates",
))

_SAFE_NAME_RE = re.compile(r"^[a-zA-Z0-9][a-zA-Z0-9_ -]*$")


def _sanitize_template_name(name: str) -> str:
    """Validate and return a safe template name, or raise ValueError."""
    name = name.strip()
    if not name:
        raise ValueError("Template name must not be empty")
    if not _SAFE_NAME_RE.match(name):
        raise ValueError(
            f"Invalid template name: {name!r}  "
            "(only letters, digits, underscores, hyphens, and spaces)"
        )
    if len(name) > 100:
        raise ValueError("Template name too long (max 100 chars)")
    return name


def list_templates() -> list[str]:
    """Return sorted list of saved template names."""
    if not TEMPLATES_DIR.is_dir():
        return []
    return sorted(
        p.stem for p in TEMPLATES_DIR.glob("*.json")
    )


def save_template(name: str, data: dict[str, Any]) -> None:
    """Save template data to disk."""
    name = _sanitize_template_name(name)
    TEMPLATES_DIR.mkdir(parents=True, exist_ok=True)
    path = TEMPLATES_DIR / f"{name}.json"
    with open(path, "w") as f:
        json.dump(data, f, indent=2)


def load_template(name: str) -> dict[str, Any] | None:
    """Load template data from disk."""
    name = _sanitize_template_name(name)
    path = TEMPLATES_DIR / f"{name}.json"
    if not path.is_file():
        return None
    try:
        with open(path) as f:
            return json.load(f)
    except (json.JSONDecodeError, OSError):
        return None


def delete_template(name: str) -> bool:
    """Delete a saved template."""
    name = _sanitize_template_name(name)
    path = TEMPLATES_DIR / f"{name}.json"
    if path.is_file():
        path.unlink()
        return True
    return False


class SaveTemplateScreen(ModalScreen[str | None]):
    """Modal to save current form state as a named template."""

    BINDINGS = [
        Binding("escape", "cancel", "Cancel", show=False),
    ]

    DEFAULT_CSS = """
    SaveTemplateScreen {
        align: center middle;
    }
    #save-dialog {
        width: 50;
        height: auto;
        border: thick $accent;
        background: $surface;
        padding: 1 2;
    }
    #save-title {
        text-style: bold;
        margin-bottom: 1;
    }
    #save-input {
        margin-bottom: 1;
    }
    #save-buttons {
        height: 3;
        align: center middle;
    }
    #save-buttons Button {
        margin: 0 1;
        min-width: 10;
    }
    """

    def compose(self) -> ComposeResult:
        with Vertical(id="save-dialog"):
            yield Static("[b]Save Template[/b]", id="save-title", markup=True)
            yield Input(placeholder="Template name…", id="save-input")
            with Horizontal(id="save-buttons"):
                yield Button("Save", variant="success", id="btn-save")
                yield Button("Cancel", variant="default", id="btn-save-cancel")

    def on_button_pressed(self, event: Button.Pressed) -> None:
        if event.button.id == "btn-save":
            name = self.query_one("#save-input", Input).value.strip()
            if name:
                try:
                    _sanitize_template_name(name)
                except ValueError as e:
                    self.query_one("#save-title", Static).update(
                        f"[b red]{e}[/b red]"
                    )
                    return
                self.dismiss(name)
            return
        self.dismiss(None)

    def action_cancel(self) -> None:
        self.dismiss(None)


class LoadTemplateScreen(ModalScreen[str | None]):
    """Modal to select and load a saved template."""

    BINDINGS = [
        Binding("escape", "cancel", "Cancel", show=False),
    ]

    DEFAULT_CSS = """
    LoadTemplateScreen {
        align: center middle;
    }
    #load-dialog {
        width: 50;
        height: 60%;
        border: thick $accent;
        background: $surface;
        padding: 1 2;
    }
    #load-title {
        text-style: bold;
        margin-bottom: 1;
    }
    #load-list {
        height: 1fr;
    }
    .tmpl-row {
        height: auto;
        margin-bottom: 0;
    }
    .tmpl-name {
        width: 1fr;
    }
    .tmpl-del {
        width: auto;
        min-width: 5;
        background: $error;
        color: $text;
    }
    #btn-load-cancel {
        margin-top: 1;
        width: 100%;
    }
    """

    def compose(self) -> ComposeResult:
        templates = list_templates()
        with Vertical(id="load-dialog"):
            yield Static("[b]Load Template[/b]", id="load-title", markup=True)
            with VerticalScroll(id="load-list"):
                if not templates:
                    yield Static("[dim]No saved templates[/dim]", markup=True)
                for name in templates:
                    with Horizontal(classes="tmpl-row"):
                        yield Button(
                            name, variant="default",
                            id=f"tmpl-{name}", classes="tmpl-name",
                        )
                        yield Button(
                            "X", variant="error",
                            id=f"del-{name}",
                            classes="tmpl-del",
                        )
            yield Button("Cancel", variant="default", id="btn-load-cancel")

    def on_button_pressed(self, event: Button.Pressed) -> None:
        bid = event.button.id or ""
        if bid == "btn-load-cancel":
            self.dismiss(None)
            return
        if bid.startswith("tmpl-"):
            self.dismiss(bid[5:])
            return
        if bid.startswith("del-"):
            name = bid[4:]
            from slurm_term.screens.confirm import ConfirmScreen
            self.app.push_screen(
                ConfirmScreen(f"Delete template [b]{name}[/b]?"),
                callback=lambda ok, n=name: self._do_delete(n) if ok else None,
            )

    def _do_delete(self, name: str) -> None:
        """Delete the template and rebuild the list in-place."""
        delete_template(name)
        self._refresh_list()

    def _refresh_list(self) -> None:
        """Rebuild the template list without closing the modal."""
        scroll = self.query_one("#load-list", VerticalScroll)
        scroll.remove_children()
        templates = list_templates()
        if not templates:
            scroll.mount(Static("[dim]No saved templates[/dim]", markup=True))
        for name in templates:
            row = Horizontal(classes="tmpl-row")
            scroll.mount(row)
            row.mount(Button(
                name, variant="default",
                id=f"tmpl-{name}", classes="tmpl-name",
            ))
            row.mount(Button(
                "X", variant="error",
                id=f"del-{name}",
                classes="tmpl-del",
            ))

    def action_cancel(self) -> None:
        self.dismiss(None)
