"""Slurm API wrapper — abstracts all subprocess calls to Slurm CLI tools.

Every shell interaction goes through :meth:`SlurmController._run` so that
the entire module is trivially mockable in tests.

Safety: All commands use ``subprocess.run`` with list args (no shell=True).
Job IDs are validated as numeric before being passed to any command.
"""

from __future__ import annotations

import json
import getpass
import re
import subprocess
import sys
import threading
from dataclasses import dataclass, field
from typing import Any


@dataclass
class JobInfo:
    """Normalised representation of a single Slurm job."""

    job_id: str
    name: str
    partition: str
    state: str
    time_used: str
    nodes: str
    reason: str
    user: str = ""
    work_dir: str = ""
    stdout_path: str = ""
    stderr_path: str = ""
    submit_time: str = ""
    node_list: str = ""
    extra: dict[str, Any] = field(default_factory=dict)


# Pattern for valid Slurm job IDs (numeric, optionally with array suffix)
_JOB_ID_RE = re.compile(r"^\d+(_\d+)?$")

# Pattern for safe sbatch parameter keys
_SAFE_KEY_RE = re.compile(r"^[a-zA-Z][a-zA-Z0-9_-]*$")


def _validate_job_id(job_id: str) -> str:
    """Validate and return a job ID, or raise ValueError."""
    job_id = str(job_id).strip()
    if not _JOB_ID_RE.match(job_id):
        raise ValueError(f"Invalid job ID: {job_id!r}")
    return job_id


def _validate_param_value(value: str) -> str:
    """Reject parameter values containing null bytes or newlines."""
    if "\x00" in value or "\n" in value or "\r" in value:
        raise ValueError(f"Parameter value contains invalid characters: {value!r}")
    return value


# Pattern for safe sacct filter values (user names, time specs)
_SAFE_FILTER_RE = re.compile(r"^[a-zA-Z0-9_.@:+/-]+$")


class SlurmController:
    """Thin wrapper around Slurm CLI tools."""

    @staticmethod
    def _run(
        cmd: list[str], *, check: bool = False, timeout: float = 30.0,
    ) -> subprocess.CompletedProcess[str]:
        """Execute *cmd* via subprocess (never shell=True).

        The default ``check=False`` means failures never raise — callers
        should inspect ``result.returncode`` instead.

        Returns a synthetic failed result if the binary is not found or
        if the command times out (rc=124, matching GNU ``timeout``).
        """
        try:
            return subprocess.run(
                cmd, capture_output=True, text=True, check=check,
                timeout=timeout,
            )
        except subprocess.TimeoutExpired:
            return subprocess.CompletedProcess(
                args=cmd, returncode=124, stdout="",
                stderr=f"{cmd[0]}: timed out after {timeout}s",
            )
        except FileNotFoundError:
            return subprocess.CompletedProcess(
                args=cmd, returncode=127, stdout="",
                stderr=f"{cmd[0]}: command not found",
            )

    @staticmethod
    def current_user() -> str:
        return getpass.getuser()

    # ----- queue -----------------------------------------------------------

    def get_queue(self, user: str | None = None) -> list[JobInfo]:
        if user is None:
            user = self.current_user()
        result = self._run(["squeue", "-u", user, "--json"], check=False)
        if result.returncode != 0:
            return []
        try:
            data = json.loads(result.stdout)
        except json.JSONDecodeError:
            return []
        return [self._parse_job_entry(e) for e in data.get("jobs", [])]

    # ----- partitions ------------------------------------------------------

    def get_partitions(self) -> list[str]:
        result = self._run(["sinfo", "-h", "-o", "%P"], check=False)
        if result.returncode != 0:
            return []
        return [
            line.strip().rstrip("*")
            for line in result.stdout.strip().splitlines()
            if line.strip()
        ]

    # ----- job details -----------------------------------------------------

    def get_job_details(self, job_id: str) -> dict[str, Any]:
        job_id = _validate_job_id(job_id)
        result = self._run(
            ["scontrol", "show", "job", job_id, "--json"], check=False,
        )
        if result.returncode != 0:
            return {}
        try:
            data = json.loads(result.stdout)
        except json.JSONDecodeError:
            return {}
        jobs = data.get("jobs", [])
        return jobs[0] if jobs else {}

    # ----- actions ---------------------------------------------------------

    def cancel_job(self, job_id: str) -> bool:
        job_id = _validate_job_id(job_id)
        return self._run(["scancel", job_id], check=False).returncode == 0

    def hold_job(self, job_id: str) -> bool:
        job_id = _validate_job_id(job_id)
        return self._run(["scontrol", "hold", job_id], check=False).returncode == 0

    def release_job(self, job_id: str) -> bool:
        job_id = _validate_job_id(job_id)
        return self._run(["scontrol", "release", job_id], check=False).returncode == 0

    def submit_job(self, script_path: str, params: dict[str, str] | None = None) -> str:
        """Submit a job via sbatch.  Returns the new job ID.

        All parameters are passed as list elements (never through a shell).
        Parameter keys and values are validated before being passed.
        """
        cmd: list[str] = ["sbatch"]
        if params:
            for key, value in params.items():
                if not _SAFE_KEY_RE.match(key):
                    raise ValueError(f"Unsafe parameter key: {key!r}")
                _validate_param_value(value)
                if value:
                    cmd.append(f"--{key}={value}")
                else:
                    cmd.append(f"--{key}")
        # Prevent script path from being interpreted as a flag
        if script_path.startswith("-"):
            script_path = f"./{script_path}"
        cmd.append(script_path)

        result = self._run(cmd, check=False)
        if result.returncode != 0:
            raise RuntimeError(
                f"sbatch failed (rc={result.returncode}): {result.stderr.strip()}"
            )
        parts = result.stdout.strip().split()
        if parts:
            return parts[-1]
        raise RuntimeError(f"Unexpected sbatch output: {result.stdout!r}")

    def srun(
        self, cmd: list[str], params: dict[str, str] | None = None
    ) -> tuple[int, str]:
        """Launch an interactive srun session (blocking, foreground).

        Uses ``subprocess.Popen`` (no shell) for safety.  stderr is piped
        through a reader thread so it is both printed to the terminal in
        real-time **and** captured.  Returns ``(returncode, stderr_text)``
        so the caller can surface error details in the TUI.
        """
        srun_cmd: list[str] = ["srun"]
        if params:
            for key, value in params.items():
                if not _SAFE_KEY_RE.match(key):
                    raise ValueError(f"Unsafe parameter key: {key!r}")
                _validate_param_value(value)
                if value:
                    srun_cmd.append(f"--{key}={value}")
                else:
                    srun_cmd.append(f"--{key}")
        for arg in cmd:
            _validate_param_value(arg)
        srun_cmd.extend(cmd)

        # Pipe stderr so we can capture it while still showing it in
        # real-time via a reader thread.  stdin/stdout stay connected to
        # the terminal for full interactive use.
        stderr_chunks: list[str] = []

        proc = subprocess.Popen(srun_cmd, stderr=subprocess.PIPE)

        def _read_stderr() -> None:
            assert proc.stderr is not None
            for raw_line in proc.stderr:
                decoded = raw_line.decode("utf-8", errors="replace")
                stderr_chunks.append(decoded)
                sys.stderr.write(decoded)
                sys.stderr.flush()

        reader = threading.Thread(target=_read_stderr, daemon=True)
        reader.start()
        proc.wait()
        reader.join(timeout=2)

        return proc.returncode, "".join(stderr_chunks).strip()

    # ----- cluster info ----------------------------------------------------

    def get_cluster_name(self) -> str:
        result = self._run(["scontrol", "show", "config"])
        if result.returncode != 0:
            return "unknown"
        for line in result.stdout.splitlines():
            key_val = line.split("=", 1)
            if len(key_val) == 2 and key_val[0].strip() == "ClusterName":
                return key_val[1].strip()
        return "unknown"

    def get_sinfo(self) -> list[dict[str, str]]:
        """Return partition/node summary rows from ``sinfo``.

        Each row is a dict with keys: partition, avail, timelimit, nodes,
        state, nodelist, cpus, memory, gres.
        """
        fmt = "%P|%a|%l|%D|%T|%N|%c|%m|%G"
        result = self._run(["sinfo", "-h", "-o", fmt])
        if result.returncode != 0:
            return []
        keys = ["partition", "avail", "timelimit", "nodes", "state",
                "nodelist", "cpus", "memory", "gres"]
        rows: list[dict[str, str]] = []
        for line in result.stdout.strip().splitlines():
            parts = line.split("|")
            if len(parts) >= len(keys):
                row = {k: parts[i].strip().rstrip("*") for i, k in enumerate(keys)}
                rows.append(row)
        return rows

    # Regex to parse scontrol key=value pairs where values may contain spaces.
    # Matches "Key=Value" where Value extends until the next "Word=" or end of line.
    _SCONTROL_KV_RE = re.compile(r"(\w+)=(.*?)(?=\s+\w+=|$)")

    def get_node_info(self) -> list[dict[str, str]]:
        """Return per-node hardware details from ``scontrol show nodes``.

        Each dict has keys like NodeName, CPUTot, RealMemory, Gres, State, etc.
        """
        result = self._run(["scontrol", "show", "nodes"])
        if result.returncode != 0:
            return []
        nodes: list[dict[str, str]] = []
        current: dict[str, str] = {}
        for line in result.stdout.splitlines():
            line = line.strip()
            if not line:
                if current:
                    nodes.append(current)
                    current = {}
                continue
            for match in self._SCONTROL_KV_RE.finditer(line):
                current[match.group(1)] = match.group(2).strip()
        if current:
            nodes.append(current)
        return nodes

    # ----- accounting / metrics --------------------------------------------

    def get_sacct(
        self, user: str | None = None, start_time: str | None = None,
    ) -> list[dict[str, str]]:
        """Return completed-job accounting data from ``sacct``.

        Each row is a dict with keys: job_id, name, partition, state,
        elapsed, total_cpu, max_rss, exit_code.
        """
        cmd = [
            "sacct", "-n", "-P",
            "--format=JobID,JobName,Partition,State,Elapsed,TotalCPU,MaxRSS,ExitCode",
        ]
        if user:
            if not _SAFE_FILTER_RE.match(user):
                raise ValueError(f"Invalid user filter: {user!r}")
            cmd.extend(["-u", user])
        if start_time:
            if not _SAFE_FILTER_RE.match(start_time):
                raise ValueError(f"Invalid time filter: {start_time!r}")
            cmd.extend(["-S", start_time])
        result = self._run(cmd, check=False)
        if result.returncode != 0:
            return []
        keys = ["job_id", "name", "partition", "state",
                "elapsed", "total_cpu", "max_rss", "exit_code"]
        rows: list[dict[str, str]] = []
        for line in result.stdout.strip().splitlines():
            parts = line.split("|")
            if len(parts) >= len(keys):
                row = {k: parts[i].strip() for i, k in enumerate(keys)}
                # Skip sub-steps (e.g. "12345.batch") — keep only main jobs
                if "." not in row["job_id"]:
                    rows.append(row)
        return rows

    def get_sstat(self, job_id: str) -> dict[str, str]:
        """Return live resource usage for a running job via ``sstat``.

        Returns dict with keys: avg_cpu, max_rss, max_vmsize, or empty
        dict on failure.

        Tries the bare job ID first; if that yields no data the
        ``.batch`` step suffix is appended automatically (required by
        most Slurm versions for batch jobs).
        """
        job_id = _validate_job_id(job_id)
        for suffix in ("", ".batch"):
            result = self._run(
                ["sstat", "-n", "-P", "--format=AveCPU,MaxRSS,MaxVMSize",
                 "-j", f"{job_id}{suffix}"],
                check=False,
            )
            if result.returncode != 0:
                continue
            for line in result.stdout.strip().splitlines():
                parts = line.split("|")
                if len(parts) >= 3 and any(p.strip() for p in parts[:3]):
                    return {
                        "avg_cpu": parts[0].strip(),
                        "max_rss": parts[1].strip(),
                        "max_vmsize": parts[2].strip(),
                    }
        return {}

    # ----- GPU metrics -----------------------------------------------------

    def get_gpu_utilization(self, node: str | None = None) -> list[float]:
        """Query GPU utilization via nvidia-smi.

        Returns a list of utilization percentages (one per GPU), or an
        empty list on failure.  If *node* is given the command is run
        via ``ssh`` on the target node.
        """
        base_cmd = [
            "nvidia-smi",
            "--query-gpu=utilization.gpu",
            "--format=csv,noheader,nounits",
        ]
        cmd = ["ssh", node] + base_cmd if node else base_cmd
        result = self._run(cmd, timeout=10.0)
        if result.returncode != 0:
            return []
        values: list[float] = []
        for line in result.stdout.strip().splitlines():
            line = line.strip()
            if line:
                try:
                    values.append(float(line))
                except ValueError:
                    pass
        return values

    # ----- parsing ---------------------------------------------------------

    @staticmethod
    def _parse_job_entry(entry: dict[str, Any]) -> JobInfo:
        time_raw = entry.get("time", {})
        if isinstance(time_raw, dict):
            elapsed = time_raw.get("elapsed", 0)
            h, rem = divmod(int(elapsed), 3600)
            m, s = divmod(rem, 60)
            time_used = f"{h:02d}:{m:02d}:{s:02d}"
        else:
            time_used = str(time_raw)

        state_raw = entry.get("job_state", "UNKNOWN")
        state = state_raw[0] if isinstance(state_raw, list) else str(state_raw)

        return JobInfo(
            job_id=str(entry.get("job_id", "")),
            name=str(entry.get("name", "")),
            partition=str(entry.get("partition", "")),
            state=state or "UNKNOWN",
            time_used=time_used,
            nodes=str(
                entry.get("nodes", "")
                or (entry.get("node_count", {}).get("number", "")
                    if isinstance(entry.get("node_count"), dict) else
                    entry.get("node_count", ""))
            ),
            reason=str(entry.get("state_reason", "")),
            user=str(entry.get("user_name", "")),
            work_dir=str(entry.get("working_directory", "")),
            stdout_path=str(entry.get("standard_output", "")),
            stderr_path=str(entry.get("standard_error", "")),
            submit_time=str(
                entry.get("submit_time", {}).get("number", "")
                if isinstance(entry.get("submit_time"), dict)
                else entry.get("submit_time", "")
            ),
            node_list=str(entry.get("nodes", "")),
        )
