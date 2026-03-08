# SlurmTerm

A keyboard-driven Terminal User Interface for the [Slurm](https://slurm.schedmd.com/) workload manager.

Monitor jobs, compose batch scripts or interactive sessions, explore cluster hardware, and browse job history — all without leaving your terminal.

> From **v0.1.4**: Rewritten in Rust for instant startup (~2 ms), zero Python dependencies, and a single static binary. Same `slurm-term` command, same feature set.

## Installation

```bash
pip install slurm-term
```

Or via cargo:

```bash
cargo install slurm-term
```

No Python runtime needed — the package ships a compiled native binary.

## Quick Start

```bash
slurm-term                          # on a Slurm login node
slurm-term --demo                   # simulated cluster (no Slurm required)
slurm-term --file job.sbatch        # load a .sbatch file into the Composer
slurm-term --since now-3days        # set initial history window
```

## Features

### Monitor

Auto-refreshing job queue with color-coded states. Cancel, hold, and release jobs directly. Supports multi-select (`Space`) for bulk actions, searchable filter (`/`), and inline job inspection (`Enter`). Press `s` to resubmit a job.

### Composer

Unified job submission form supporting both **sbatch** (batch) and **srun** (interactive) modes:

- Live script preview that updates as you type, with full cursor-based editing
- Core resource fields with inline validation: partition, time, nodes, tasks, CPUs, memory, GPUs
- Searchable catalog of 40+ additional Slurm parameters with built-in documentation (`?`)
- Save and load job templates (`Ctrl+T` / `Ctrl+L`)
- Import existing `.sbatch` files (`Ctrl+O` or `--file` CLI flag)
- Direct preview editing with bi-directional sync to form fields

### Hardware

Cluster overview with two sub-tabs:

- **Partitions**: availability, time limits, node counts, CPUs, memory, GRES
- **Nodes**: per-node state, CPU count, memory, free memory, GRES, load

### History

Completed job accounting from `sacct` with configurable time window (1–30 days). Displays elapsed time, CPU usage, peak memory, and exit codes.

## Key Bindings

| Key        | Action                      |
|------------|-----------------------------|
| `1`–`4`    | Switch tab                  |
| `q`        | Quit                        |
| `/`        | Search jobs (Monitor)       |
| `Enter`    | Inspect / edit              |
| `Space`    | Select job (Monitor)        |
| `k`        | Kill selected jobs          |
| `Tab`      | Switch pane / view          |
| `?`        | Parameter help (Composer)   |
| `a`        | Add parameter (Composer)    |
| `Ctrl+O`   | Load .sbatch file           |
| `Ctrl+S`   | Submit job                  |
| `Ctrl+T`   | Save template               |
| `Ctrl+L`   | Load template               |
| `</>`     | Change time window (History)|
| `r`        | Refresh                     |

## Configuration

Configuration is read from `~/.config/slurmterm/config.toml`:

```toml
[poll]
monitor = 3.0
hardware = 30.0
history = 60.0

[general]
history_window = "now-7days"
```

## License

MIT
