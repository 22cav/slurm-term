"""Mock Slurm controller for demo / local development.

Simulates a small HPC cluster with jobs that transition through states
over time. Creates real temp log files so the Inspector tab works.

Usage::

    python -m slurm_term.main --demo
"""

from __future__ import annotations

import atexit
import os
import random
import shutil
import subprocess
import tempfile
import time
from datetime import datetime, timedelta
from typing import Any

from slurm_term.slurm_api import SlurmController, JobInfo, _validate_job_id

_PARTITIONS = ["debug", "batch", "gpu", "bigmem"]

_JOB_NAMES = [
    "train_resnet50", "preprocess_data", "eval_model", "hyperopt_search",
    "feature_extract", "run_simulation", "postprocess", "benchmark_v2",
    "data_augment", "inference_batch",
]

_USERS = ["matte", "alice", "bob"]

_REASONS = ["None", "Resources", "Priority", "QOSMaxJobsPerUserLimit", "Dependency"]

# Fake log lines that get appended to simulate output
_LOG_LINES = [
    "Epoch {n}/100 - loss: {l:.4f} - val_loss: {v:.4f} - lr: 0.001",
    "Processing batch {n}/500 [{p}%] ETA: {t}s",
    "Checkpoint saved to /scratch/model_epoch_{n}.pt",
    "GPU memory: {m}MB / 16384MB  |  Utilization: {u}%",
    "INFO: Worker {w} finished task {n} in {t:.2f}s",
    "Loading dataset shard {n}/10 ({s}MB)",
    "Evaluating on validation set... accuracy: {a:.2f}%",
]

# Resource profiles per partition
_RESOURCE_PROFILES: dict[str, dict[str, Any]] = {
    "debug":  {"cpus_per_task": "4",  "memory_mb": "16000", "gres": ""},
    "batch":  {"cpus_per_task": "16", "memory_mb": "64000", "gres": ""},
    "gpu":    {"cpus_per_task": "8",  "memory_mb": "32000", "gres": "gpu:a100:1"},
    "bigmem": {"cpus_per_task": "32", "memory_mb": "256000", "gres": ""},
}


class MockSlurmController(SlurmController):
    """Fake Slurm controller that simulates cluster activity."""

    def __init__(self, num_jobs: int = 8, seed: int | None = 42) -> None:
        self._rng = random.Random(seed)
        self._next_id = 100001
        self._jobs: dict[str, dict[str, Any]] = {}
        self._cancelled: set[str] = set()
        self._held: set[str] = set()
        self._start_time = datetime.now()

        # Temp directory for fake log files
        self._tmpdir = tempfile.mkdtemp(prefix="slurmterm_demo_")
        atexit.register(lambda: shutil.rmtree(self._tmpdir, ignore_errors=True))

        for _ in range(num_jobs):
            self._spawn_job()

    # -- overrides ----------------------------------------------------------

    @staticmethod
    def _run(cmd: list[str], *, check: bool = False) -> subprocess.CompletedProcess[str]:
        return subprocess.CompletedProcess(cmd, 0, "", "")

    @staticmethod
    def current_user() -> str:
        return os.environ.get("USER", "matte")

    def get_cluster_name(self) -> str:
        return "demo-cluster"

    def get_queue(self, user: str | None = None) -> list[JobInfo]:
        self._tick()
        return [self._to_job_info(j) for j in self._jobs.values()]

    def get_partitions(self) -> list[str]:
        return list(_PARTITIONS)

    def cancel_job(self, job_id: str) -> bool:
        _validate_job_id(job_id)
        if job_id in self._jobs:
            self._cancelled.add(job_id)
            self._jobs[job_id]["state"] = "CANCELLED"
            return True
        return False

    def hold_job(self, job_id: str) -> bool:
        _validate_job_id(job_id)
        if job_id in self._jobs and self._jobs[job_id]["state"] == "PENDING":
            self._held.add(job_id)
            return True
        return False

    def release_job(self, job_id: str) -> bool:
        _validate_job_id(job_id)
        if job_id in self._held:
            self._held.discard(job_id)
            return True
        return False

    def get_job_details(self, job_id: str) -> dict[str, Any]:
        job = self._jobs.get(job_id)
        if not job:
            return {}
        log_path = job.get("log_path", "")
        profile = _RESOURCE_PROFILES.get(job["partition"], _RESOURCE_PROFILES["batch"])
        submit_ts = self._start_time - timedelta(seconds=self._rng.randint(60, 3600))
        return {
            "job_id": job_id,
            "name": job["name"],
            "job_state": job["state"],
            "partition": job["partition"],
            "user_name": job["user"],
            "working_directory": f"/home/{job['user']}/projects/{job['name']}",
            "submit_time": submit_ts.strftime("%Y-%m-%dT%H:%M:%S"),
            "nodes": job.get("node_list", "node001"),
            "standard_output": log_path,
            "standard_error": log_path.replace(".out", ".err") if log_path else "",
            "time_limit": job.get("time_limit", 3600),
            "run_time": job.get("elapsed", 0),
            "cpus_per_task": profile["cpus_per_task"],
            "minimum_memory_per_node": profile["memory_mb"],
            "gres_detail": profile["gres"],
            "slurmterm_metrics": job.get("metrics", {}),
        }

    def submit_job(self, script_path: str, params: dict[str, str] | None = None) -> str:
        job = self._spawn_job(state="PENDING")
        return job["id"]

    def srun(
        self, cmd: list[str], params: dict[str, str] | None = None
    ) -> tuple[int, str]:
        """Mock srun — prints a message and returns ``(0, "")``."""
        print(f"[demo] srun {' '.join(cmd)} (simulated, press Ctrl-D to return)")
        import sys
        try:
            for line in sys.stdin:
                pass
        except (EOFError, KeyboardInterrupt):
            pass
        return 0, ""

    def get_sinfo(self) -> list[dict[str, str]]:
        return [
            {"partition": "debug", "avail": "up", "timelimit": "00:30:00",
             "nodes": "4", "state": "idle", "nodelist": "node[001-004]",
             "cpus": "16", "memory": "64000", "gres": "(null)"},
            {"partition": "batch", "avail": "up", "timelimit": "7-00:00:00",
             "nodes": "20", "state": "mixed", "nodelist": "node[005-024]",
             "cpus": "64", "memory": "256000", "gres": "(null)"},
            {"partition": "batch", "avail": "up", "timelimit": "7-00:00:00",
             "nodes": "8", "state": "allocated", "nodelist": "node[025-032]",
             "cpus": "64", "memory": "256000", "gres": "(null)"},
            {"partition": "gpu", "avail": "up", "timelimit": "3-00:00:00",
             "nodes": "8", "state": "mixed", "nodelist": "gpu[001-008]",
             "cpus": "32", "memory": "128000", "gres": "gpu:a100:4"},
            {"partition": "gpu", "avail": "up", "timelimit": "3-00:00:00",
             "nodes": "4", "state": "idle", "nodelist": "gpu[009-012]",
             "cpus": "32", "memory": "128000", "gres": "gpu:a100:4"},
            {"partition": "bigmem", "avail": "up", "timelimit": "2-00:00:00",
             "nodes": "2", "state": "idle", "nodelist": "bigmem[001-002]",
             "cpus": "128", "memory": "1024000", "gres": "(null)"},
        ]

    def get_node_info(self) -> list[dict[str, str]]:
        nodes = []
        for i in range(1, 5):
            nodes.append({
                "NodeName": f"node{i:03d}", "State": "IDLE", "CPUTot": "16",
                "RealMemory": "64000", "Gres": "(null)", "Partitions": "debug",
                "CPULoad": f"{self._rng.uniform(0, 1):.2f}", "FreeMem": "62000",
            })
        for i in range(5, 33):
            state = self._rng.choice(["MIXED", "ALLOCATED", "IDLE"])
            load = f"{self._rng.uniform(10, 60):.2f}" if state != "IDLE" else "0.00"
            fm = str(self._rng.randint(50000, 250000))
            nodes.append({
                "NodeName": f"node{i:03d}", "State": state, "CPUTot": "64",
                "RealMemory": "256000", "Gres": "(null)", "Partitions": "batch",
                "CPULoad": load, "FreeMem": fm,
            })
        for i in range(1, 13):
            state = self._rng.choice(["MIXED", "IDLE"])
            load = f"{self._rng.uniform(5, 30):.2f}" if state == "MIXED" else "0.00"
            fm = str(self._rng.randint(60000, 120000))
            nodes.append({
                "NodeName": f"gpu{i:03d}", "State": state, "CPUTot": "32",
                "RealMemory": "128000", "Gres": "gpu:a100:4", "Partitions": "gpu",
                "CPULoad": load, "FreeMem": fm,
            })
        for i in range(1, 3):
            nodes.append({
                "NodeName": f"bigmem{i:03d}", "State": "IDLE", "CPUTot": "128",
                "RealMemory": "1024000", "Gres": "(null)", "Partitions": "bigmem",
                "CPULoad": "0.00", "FreeMem": "1020000",
            })
        return nodes

    def get_sacct(
        self, user: str | None = None, start_time: str | None = None,
    ) -> list[dict[str, str]]:
        """Return fake completed-job history."""
        rows: list[dict[str, str]] = []
        # Generate some historical completed jobs
        for i in range(15):
            jid = str(99900 + i)
            elapsed_s = self._rng.randint(120, 86400)
            h, rem = divmod(elapsed_s, 3600)
            m, s = divmod(rem, 60)
            elapsed = f"{h:02d}:{m:02d}:{s:02d}"
            cpu_eff = self._rng.uniform(0.3, 1.0)
            cpu_s = int(elapsed_s * cpu_eff * self._rng.randint(1, 16))
            cpu_h, cpu_rem = divmod(cpu_s, 3600)
            cpu_m, cpu_sec = divmod(cpu_rem, 60)
            total_cpu = f"{cpu_h:02d}:{cpu_m:02d}:{cpu_sec:02d}"
            max_rss = f"{self._rng.randint(500, 64000)}M"
            state = self._rng.choice(
                ["COMPLETED"] * 6 + ["FAILED"] * 2 + ["TIMEOUT", "CANCELLED"]
            )
            exit_code = "0:0" if state == "COMPLETED" else f"{self._rng.randint(1, 127)}:0"
            rows.append({
                "job_id": jid,
                "name": self._rng.choice(_JOB_NAMES),
                "partition": self._rng.choice(_PARTITIONS),
                "state": state,
                "elapsed": elapsed,
                "total_cpu": total_cpu,
                "max_rss": max_rss,
                "exit_code": exit_code,
            })
        return rows

    def get_sstat(self, job_id: str) -> dict[str, str]:
        """Return fake live metrics for a running job."""
        job = self._jobs.get(job_id)
        if not job or job["state"] != "RUNNING":
            return {}
        metrics = job.get("metrics", {})
        cpu_pct = metrics.get("cpu", [50.0])[-1]
        mem_mb = int(metrics.get("mem", [50.0])[-1] / 100 * 32000)
        return {
            "avg_cpu": f"{cpu_pct:.0f}%",
            "max_rss": f"{mem_mb}M",
            "max_vmsize": f"{mem_mb + self._rng.randint(1000, 5000)}M",
        }

    def _spawn_job(self, state: str | None = None) -> dict[str, Any]:
        job_id = str(self._next_id)
        self._next_id += 1

        if state is None:
            state = self._rng.choice(["RUNNING"] * 3 + ["PENDING"] * 2 + ["COMPLETING"])

        # Create a real log file
        log_path = os.path.join(self._tmpdir, f"slurm-{job_id}.out")
        err_path = log_path.replace(".out", ".err")
        with open(log_path, "w") as f:
            f.write(f"=== SLURM Job {job_id} ===\n")
            f.write(f"Started at: {datetime.now().isoformat(timespec='seconds')}\n")
            f.write(f"Working directory: /home/matte/project\n\n")
        with open(err_path, "w") as f:
            f.write(f"=== SLURM Job {job_id} stderr ===\n")

        job = {
            "id": job_id,
            "name": self._rng.choice(_JOB_NAMES),
            "partition": self._rng.choice(_PARTITIONS),
            "state": state,
            "user": self._rng.choice(_USERS),
            "elapsed": self._rng.randint(0, 36000) if state == "RUNNING" else 0,
            "nodes": str(self._rng.randint(1, 8)),
            "reason": "None" if state == "RUNNING" else self._rng.choice(_REASONS),
            "node_list": f"node[{self._rng.randint(1, 50):03d}-{self._rng.randint(51, 100):03d}]",
            "born": time.monotonic(),
            "log_path": log_path,
            "log_counter": 0,
            "time_limit": self._rng.choice([3600, 7200, 14400, 86400]),
            "metrics": self._generate_initial_metrics(),
        }
        self._jobs[job_id] = job
        return job

    def _generate_initial_metrics(self) -> dict[str, list[float]]:
        """Generate realistic initial metric histories with smooth random walks."""
        metrics: dict[str, list[float]] = {}
        for key, base, spread in [("cpu", 55.0, 25.0), ("mem", 50.0, 20.0), ("gpu", 45.0, 30.0)]:
            val = self._rng.uniform(base - spread, base + spread)
            history = [val]
            for _ in range(29):
                val = max(0.0, min(100.0, val + self._rng.uniform(-5.0, 5.0)))
                history.append(val)
            metrics[key] = history
        return metrics

    def _tick(self) -> None:
        now = time.monotonic()
        to_remove: list[str] = []

        for job_id, job in self._jobs.items():
            if job_id in self._cancelled or job_id in self._held:
                continue

            age = now - job["born"]

            if job["state"] == "RUNNING":
                job["elapsed"] += 3
                # Update metrics with a random walk
                for m_key in ["cpu", "mem", "gpu"]:
                    last_val = job["metrics"][m_key][-1]
                    new_val = max(0.0, min(100.0, last_val + self._rng.uniform(-10.0, 10.0)))
                    job["metrics"][m_key].append(new_val)
                    if len(job["metrics"][m_key]) > 60:
                        job["metrics"][m_key].pop(0)

                # Append a log line
                self._write_log_line(job)
                if age > 20 and self._rng.random() < 0.08:
                    job["state"] = self._rng.choice(["COMPLETED"] * 2 + ["FAILED", "TIMEOUT"])
                elif age > 15 and self._rng.random() < 0.05:
                    job["state"] = "COMPLETING"

            elif job["state"] == "COMPLETING":
                if self._rng.random() < 0.4:
                    job["state"] = "COMPLETED"

            elif job["state"] == "PENDING":
                if age > 10 and self._rng.random() < 0.15:
                    job["state"] = "RUNNING"

            if job["state"] in ("COMPLETED", "FAILED", "TIMEOUT", "CANCELLED"):
                if age > 30:
                    to_remove.append(job_id)

        for jid in to_remove:
            del self._jobs[jid]
            self._cancelled.discard(jid)

        if len(self._jobs) < 12 and self._rng.random() < 0.2:
            self._spawn_job()

    def _write_log_line(self, job: dict[str, Any]) -> None:
        """Append a simulated log line to the job's log file."""
        job["log_counter"] += 1
        n = job["log_counter"]
        template = self._rng.choice(_LOG_LINES)
        line = template.format(
            n=n, l=self._rng.uniform(0.01, 2.0), v=self._rng.uniform(0.1, 2.5),
            p=min(100, n * 2), t=self._rng.randint(5, 300),
            m=self._rng.randint(2000, 15000), u=self._rng.randint(30, 99),
            w=self._rng.randint(0, 7), s=self._rng.randint(100, 5000),
            a=self._rng.uniform(60, 99),
        )
        try:
            with open(job["log_path"], "a") as f:
                f.write(line + "\n")
        except OSError:
            pass
        # Write to stderr occasionally
        if self._rng.random() < 0.1:
            err_path = job["log_path"].replace(".out", ".err")
            try:
                with open(err_path, "a") as f:
                    f.write(f"WARNING: {line}\n")
            except OSError:
                pass

    def _to_job_info(self, job: dict[str, Any]) -> JobInfo:
        elapsed = int(job.get("elapsed", 0))
        h, rem = divmod(elapsed, 3600)
        m, s = divmod(rem, 60)
        state = "PENDING (Held)" if job["id"] in self._held else job["state"]
        return JobInfo(
            job_id=job["id"], name=job["name"], partition=job["partition"],
            state=state, time_used=f"{h:02d}:{m:02d}:{s:02d}",
            nodes=job["nodes"], reason=job["reason"], user=job["user"],
        )
