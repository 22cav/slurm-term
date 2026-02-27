"""Built-in default templates seeded on first run."""

from __future__ import annotations

from slurm_term.screens.templates import TEMPLATES_DIR, save_template

DEFAULT_TEMPLATES: dict[str, dict[str, str]] = {
    "Quick CPU Job": {
        "mode": "sbatch",
        "name": "quick-test",
        "partition": "",
        "time": "00:30:00",
        "nodes": "1",
        "ntasks": "1",
        "cpus": "1",
        "memory": "4G",
        "gpus": "",
        "script": "",
        "output": "%x-%j.out",
        "error": "%x-%j.err",
        "modules": "",
        "env": "",
        "init": "",
    },
    "Multi-Node MPI": {
        "mode": "sbatch",
        "name": "mpi-job",
        "partition": "",
        "time": "04:00:00",
        "nodes": "4",
        "ntasks": "16",
        "cpus": "1",
        "memory": "8G",
        "gpus": "",
        "script": "",
        "output": "%x-%j.out",
        "error": "%x-%j.err",
        "modules": "openmpi",
        "env": "",
        "init": "srun ./my_mpi_program",
    },
    "Single GPU Training": {
        "mode": "sbatch",
        "name": "gpu-training",
        "partition": "",
        "time": "08:00:00",
        "nodes": "1",
        "ntasks": "1",
        "cpus": "4",
        "memory": "32G",
        "gpus": "1",
        "script": "",
        "output": "%x-%j.out",
        "error": "%x-%j.err",
        "modules": "cuda\npython",
        "env": "",
        "init": "python train.py",
    },
    "Large Memory Job": {
        "mode": "sbatch",
        "name": "highmem-job",
        "partition": "",
        "time": "12:00:00",
        "nodes": "1",
        "ntasks": "1",
        "cpus": "8",
        "memory": "128G",
        "gpus": "",
        "script": "",
        "output": "%x-%j.out",
        "error": "%x-%j.err",
        "modules": "",
        "env": "",
        "init": "",
    },
    "Interactive Session": {
        "mode": "srun",
        "name": "",
        "partition": "",
        "time": "01:00:00",
        "nodes": "1",
        "ntasks": "1",
        "cpus": "2",
        "memory": "8G",
        "gpus": "",
        "script": "",
        "output": "",
        "error": "",
        "modules": "",
        "env": "",
        "init": "",
    },
}


def ensure_default_templates() -> None:
    """Seed default templates if the templates directory is empty or missing.

    Only writes defaults when no user templates exist yet (first run).
    If the user later deletes a default template, it stays deleted.
    """
    if TEMPLATES_DIR.is_dir() and any(TEMPLATES_DIR.glob("*.json")):
        return  # User already has templates — don't overwrite
    for name, data in DEFAULT_TEMPLATES.items():
        save_template(name, data)
