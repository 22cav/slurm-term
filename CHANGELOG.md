# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.6]

### Added
- **File browser for script loading**: replaced the text-input file dialog with a full filesystem browser (`Ctrl+O`). Navigates directories, shows only compatible files (`.sbatch`, `.sh`, `.job`), and validates script headers before loading.
- **Full-featured text editor** in the preview pane: line numbers in gutter, word-level navigation (`Ctrl+Left/Right`), line operations (`Ctrl+K` kill line, `Ctrl+D` delete line, `Ctrl+U` clear to start), auto-indentation on Enter, and Tab/Shift+Tab for indent/dedent.
- **Multiline field editor**: Modules, Env Vars, and Init Cmds now open in the right pane with the same full editor when pressing Enter, instead of inline editing. Fields show a clean summary in the form (e.g., `numpy, scipy  [2 modules]`).
- **Native text selection in Inspector**: mouse capture is disabled when the inspector is open, allowing standard terminal drag-to-select and copy for job details and logs.

### Fixed
- **Log display corruption**: stripped ANSI escape sequences and handled carriage returns (`\r`) in log output, fixing lines overlaying each other in Follow mode.
- **N/A fields in job inspector**: correctly parse Slurm 22+ JSON object wrappers (`{"number": N, "set": bool, "infinite": bool}`) and string-encoded numbers from mock/demo mode for time, memory, and all scalar fields.
- **Form field highlight leak**: the focused form field no longer turns green when editing the preview pane; the green editing indicator now only appears on the active pane.

### Changed
- **Smarter log polling**: logs now refresh even when Follow is paused (at the main poll interval), and use file-size change detection to skip redundant re-reads when the file hasn't changed.
- **Cleaner multiline field display**: Modules, Env Vars, and Init Cmds show a compact comma-separated summary with item count instead of raw multiline text in the form.
- **Context-aware status bar**: the bottom hints now change between form-mode and editor-mode shortcuts when editing the preview pane.
- Resolved all clippy warnings.

## [0.1.5]

### Fixed
- **Fixed script submission breaking shell control structures**: jobs with `for`/`do`/`done` loops, line continuations, or multiline init commands no longer fail with syntax errors. Submission now writes a proper temp script file instead of collapsing everything into `sbatch --wrap` with `; ` joins.
- **Fixed CRLF line endings in submitted scripts**: `\r\n` line endings from pasted text or loaded files are now normalized to `\n` at all entry points (file loading, clipboard paste, parser).
- **Fixed multiline form fields disappearing**: Init Cmds, Modules, and Env Vars fields with many lines no longer overflow and vanish when focused. The display is now capped at 8 visible lines with scroll tracking and a line-count indicator.

### Added
- **Bracketed paste support**: pasting text into the Composer (both form fields and preview editor) now uses terminal bracketed paste mode, correctly handling multiline pastes with proper `\r` filtering.

### Changed
- **Faster log following**: Inspector log file refresh now runs on a separate 1-second timer (down from 3s) when follow mode is active, for near-real-time output tailing.
- Removed the `sbatch --wrap` submission path; all submissions now go through script files.

## [0.1.4]

### Fixed
- Fixed Linux wheel compatibility: build now targets `manylinux_2_17` (glibc 2.17+) for broader HPC cluster support.

## [0.1.3]

### Changed
- **Full rewrite in Rust** — native binary, no Python runtime or dependencies required.
- Instant startup (~2 ms vs ~1 s for the Python version).
- Single static binary installable via `pip install slurm-term` or `cargo install slurm-term`.

### Added
- **Cursor-based form editing** in Composer: arrow keys, Home/End, Delete navigate within field values.
- **`.sbatch` file loading** via `Ctrl+O` dialog or `--file` CLI argument.
- **40+ parameter catalog** with built-in documentation (`?` key) and searchable add-param dialog (`a` key).
- **Full multiline editing** for Modules, Env Vars, and Init Cmds fields with Up/Down line navigation.
- **Mouse support**: click to navigate tabs, select jobs, and interact with the form.
- **Input validation** with inline red highlighting for Time, Memory, Name, Nodes, Tasks, CPUs, GPUs fields.
- **Direct preview editing** with bidirectional sync to form fields.
- **Live log follow** in Inspector: logs auto-scroll with a `FOLLOW`/`PAUSED` indicator; press `f` to toggle, scrolling up pauses automatically.
- **Hostname awareness**: header bar shows node type (login/compute); hover to reveal full hostname.
- **Sortable tables**: press `s` to cycle sort column, `S` to reverse direction in Monitor, History, and Cluster tabs.
- **Storage display**: new Storage sub-tab in Cluster showing filesystem usage (`df -h`), with color-coded usage percentages.
- **Copy to clipboard**: `Ctrl+Y` in Composer copies the generated script to the system clipboard via OSC 52.

### Removed
- Python runtime dependency (textual, textual-plotext).
- Inspector is now inline in the Monitor tab (press `Enter` on a job) rather than a separate tab.

## [0.1.2]

### Added
- **Sbatch file import** (`Ctrl+I`): parse `.sbatch` scripts and populate the Composer form.
- **Quick-peek output** (`o`): preview the last 50 lines of a job's output from the Monitor.
- **Multi-select** (`Space` / `Ctrl+A`): bulk cancel, hold, or release multiple jobs.
- **Resubmit job** (`s`): re-populate the Composer from the Inspector or History tab.
- **History time window selector**: choose 1–30 day range directly in the History tab.
- **Improved interaction**: Error catching and improved design.

## [0.1.1]

Initial public release with Monitor, Composer, Hardware, History, and Inspector tabs, `--demo` mode, template save/load, parameter catalog, and inline validation.
