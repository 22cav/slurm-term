# SlurmTerm

A keyboard-driven Terminal User Interface for the [Slurm](https://slurm.schedmd.com/) workload manager, built with [Textual](https://textual.textualize.io/).

Monitor jobs, compose batch scripts or interactive sessions, explore cluster hardware, and inspect running jobs with real-time metrics, all without leaving your terminal.

## Installation

```bash
pip install slurm-term
```

Requires Python 3.10+ and a working Slurm installation on the target machine.

## Quick Start

```bash
slurm-term                     # on a Slurm login node
slurm-term --demo              # simulated cluster (no Slurm required)
slurm-term --since now-3days   # set initial history window
```

Launch `slurm-term` on any Slurm login or compute node. The interface opens with five tabs accessible via number keys `1`–`5`.

## Features

### Monitor

Auto-refreshing job queue with color-coded states. Cancel, hold, and release jobs directly from the table. Supports multi-select (`Space` / `Ctrl+A`) for bulk actions, searchable filter bar (`/`), and quick-peek output (`o`). Press `Enter` on any job to open the Inspector.

### Composer

Unified job submission form supporting both **sbatch** (batch) and **srun** (interactive) modes:

- Live script preview that updates as you type
- Core resource fields with inline partition summary: partition, time limit, nodes, tasks, CPUs, memory, GPUs
- Searchable catalog of 30+ additional Slurm parameters with built-in documentation
- Inline validation for time formats, memory specs, and GPU configurations
- Save and load job templates for repeated workflows
- Import existing `.sbatch` files (`Ctrl+I`) — parses directives, modules, env vars, and commands
- Five built-in default templates seeded on first run
- Auto-switch to Monitor tab after successful submission

### Hardware

Cluster overview with two sub-tabs:

- **Partitions**: availability, time limits, node counts, CPUs, memory, GRES
- **Nodes**: per-node state, CPU count, memory, free memory, GRES, load

### History

Completed job accounting from `sacct` with configurable time window (1–30 days). Displays elapsed time, CPU usage, peak memory, and exit codes. Press `s` to resubmit a completed job.

### Inspector

Visual job inspector with three sub-tabs:

- **Overview**: status badge, metadata grid (partition, user, submit time, working directory, nodes), resource summary, time remaining progress bar
- **Output & Error Logs**: live stdout/stderr tailing with toggle (`e`), log file paths display
- **Live Metrics**: real-time CPU, memory, and GPU utilization charts

Press `s` to resubmit the current job — parameters are extracted and loaded into the Composer.

## Key Bindings

| Key            | Action                                   |
| -------------- | ---------------------------------------- |
| `q`            | Quit                                     |
| `1`–`5`        | Switch tab                               |
| `Ctrl+R`       | Reload configuration                     |
| `/`            | Search / filter jobs (Monitor)           |
| `Space`        | Toggle select (Monitor)                  |
| `Ctrl+A`       | Select all (Monitor)                     |
| `r`            | Refresh current view                     |
| `k`            | Cancel selected job(s)                   |
| `h` / `u`      | Hold / release selected job(s)           |
| `o`            | Peek output (Monitor)                    |
| `i` or `Enter` | Inspect selected job                     |
| `s`            | Resubmit job (Inspector / History)       |
| `e`            | Toggle stdout/stderr (Inspector)         |
| `Escape`       | Back / clear selection                   |
| `Ctrl+S`       | Submit job or launch interactive session |
| `Ctrl+T`       | Save template                            |
| `Ctrl+L`       | Load template                            |
| `Ctrl+I`       | Import `.sbatch` file                    |
| `Ctrl+Y`       | Copy script preview                      |

## Safety

All Slurm commands use `subprocess.run` with list arguments — no shell interpolation. Job IDs, parameter keys, parameter values, and job names are validated before use. User-controlled strings are escaped before rendering. Template names are sanitized to prevent path traversal.

## Requirements

- Python >= 3.10
- [Textual](https://pypi.org/project/textual/) >= 0.90.0
- [textual-plotext](https://pypi.org/project/textual-plotext/) >= 1.0.0
- A Slurm cluster (commands: `squeue`, `scontrol`, `sbatch`, `srun`, `sacct`, `sinfo`, `sstat`)

## Links

- [Source Code](https://github.com/22cav/slurm-term)
- [Issue Tracker](https://github.com/22cav/slurm-term/issues)

## License

MIT
