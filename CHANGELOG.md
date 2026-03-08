# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
