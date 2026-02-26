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
slurm-term
```

Launch `slurm-term` on any Slurm login or compute node. The interface opens with five tabs accessible via number keys `1`–`5`.

## Features

### Monitor

Auto-refreshing job queue with color-coded states. Cancel, hold, and release jobs directly from the table. Press `Enter` on any job to open the Inspector.

### Composer

Unified job submission form supporting both **sbatch** (batch) and **srun** (interactive) modes:

- Live script preview that updates as you type
- Core resource fields: partition, time limit, nodes, tasks, CPUs, memory, GPUs
- Searchable catalog of 30+ additional Slurm parameters with built-in documentation
- Inline validation for time formats, memory specs, and GPU configurations
- Save and load job templates for repeated workflows

### Hardware

Cluster overview with two sub-tabs:

- **Partitions**: availability, time limits, node counts, CPUs, memory, GRES
- **Nodes**: per-node state, CPU count, memory, free memory, GRES, load

### History

Completed job accounting from `sacct` (last 7 days). Displays elapsed time, CPU usage, peak memory, and exit codes.

### Inspector

Visual job inspector with:

- Status badge and metadata grid (partition, user, submit time, working directory, nodes)
- Resource summary (nodes, CPUs, memory, GRES)
- Live stdout/stderr log tailing
- Real-time CPU, memory, and GPU utilization charts

## Key Bindings

| Key | Action |
| --- | --- |
| `q` | Quit |
| `1`–`5` | Switch tab |
| `r` | Refresh current view |
| `k` | Cancel selected job |
| `h` / `u` | Hold / release selected job |
| `Enter` | Inspect selected job |
| `Escape` | Return to Monitor from Inspector |
| `Ctrl+S` | Submit job or launch interactive session |
| `Ctrl+T` | Save template |
| `Ctrl+L` | Load template |

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
