use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::validators::{format_hms, format_slurm_duration};

// ---------------------------------------------------------------------------
// JobInfo
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobInfo {
    pub job_id: String,
    pub name: String,
    pub partition: String,
    pub state: String,
    pub time_used: String,
    pub nodes: String,
    pub reason: String,
    pub user: String,
    pub work_dir: String,
    pub stdout_path: String,
    pub stderr_path: String,
    pub submit_time: String,
    pub node_list: String,
    /// Array master job ID (`array_job_id`), empty for non-array jobs.
    #[serde(default)]
    pub array_job_id: String,
    /// Array task ID (`array_task_id`), empty for non-array jobs and for the
    /// pending "meta" record that still holds unstarted tasks.
    #[serde(default)]
    pub array_task_id: String,
    /// Unstarted task expression of a pending array record ("5-99%10").
    #[serde(default)]
    pub array_task_string: String,
    pub extra: HashMap<String, serde_json::Value>,
}

impl JobInfo {
    /// The ID as squeue prints it (`%i`): `123`, `120_3` for an array task,
    /// or `120_[4-99]` for the pending record of an array.
    pub fn display_id(&self) -> String {
        if self.array_job_id.is_empty() {
            return self.job_id.clone();
        }
        if !self.array_task_string.is_empty() {
            return format!("{}_[{}]", self.array_job_id, self.array_task_string);
        }
        if !self.array_task_id.is_empty() {
            return format!("{}_{}", self.array_job_id, self.array_task_id);
        }
        self.job_id.clone()
    }
}

/// Outcome of a successful sbatch call.
#[derive(Debug, Clone, PartialEq)]
pub enum SubmitOutcome {
    /// The job was queued with this ID.
    Submitted(String),
    /// `--test-only`: nothing was queued; sbatch's estimate is attached.
    TestOnly(String),
}

/// Signals the UI may send with `scancel --signal`. Restricting to a known
/// list keeps arbitrary text away from the command line.
pub const JOB_SIGNALS: &[&str] = &["USR1", "USR2", "TERM", "INT", "HUP", "CONT", "STOP", "KILL"];

// ---------------------------------------------------------------------------
// SinfoRow / SacctRow / SstatRow / NodeInfo
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct SinfoRow {
    pub partition: String,
    pub avail: String,
    pub timelimit: String,
    pub nodes: String,
    pub state: String,
    #[allow(dead_code)]
    pub nodelist: String,
    pub cpus: String,
    pub memory: String,
    pub gres: String,
}

#[derive(Debug, Clone, Default)]
pub struct SacctRow {
    pub job_id: String,
    pub name: String,
    pub partition: String,
    pub state: String,
    pub elapsed: String,
    pub total_cpu: String,
    pub max_rss: String,
    pub exit_code: String,
}

#[derive(Debug, Clone, Default)]
pub struct SstatResult {
    pub avg_cpu: String,
    pub max_rss: String,
    #[allow(dead_code)]
    pub max_vmsize: String,
}

#[derive(Debug, Clone, Default)]
pub struct NodeInfoRow {
    pub fields: HashMap<String, String>,
}

#[derive(Debug, Clone, Default)]
pub struct StorageInfo {
    pub filesystem: String,
    pub size: String,
    pub used: String,
    pub avail: String,
    pub use_pct: String,
    pub mount: String,
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

/// Job ID, optionally an array element: "123" or "123_4".
static JOB_ID_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d+(_\d+)?$").unwrap());
/// An sbatch long-option name.
static SAFE_KEY_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z][a-zA-Z0-9_-]*$").unwrap());
/// A value safe to pass as a squeue/sacct filter argument.
static SAFE_FILTER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z0-9_.@:+/-]+$").unwrap());

fn validate_job_id(job_id: &str) -> Result<String, String> {
    let job_id = job_id.trim();
    if !JOB_ID_RE.is_match(job_id) {
        return Err(format!("Invalid job ID: {job_id:?}"));
    }
    Ok(job_id.to_string())
}

fn validate_param_value(value: &str) -> Result<(), String> {
    if value.contains('\0') || value.contains('\n') || value.contains('\r') {
        return Err(format!(
            "Parameter value contains invalid characters: {value:?}"
        ));
    }
    Ok(())
}

fn validate_safe_key(key: &str) -> Result<(), String> {
    if !SAFE_KEY_RE.is_match(key) {
        return Err(format!("Unsafe parameter key: {key:?}"));
    }
    Ok(())
}

fn validate_safe_filter(value: &str) -> Result<(), String> {
    if !SAFE_FILTER_RE.is_match(value) {
        return Err(format!("Invalid filter value: {value:?}"));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// SlurmController trait
// ---------------------------------------------------------------------------

pub trait SlurmController {
    fn current_user(&self) -> String;
    fn get_cluster_name(&self) -> String;
    fn get_queue(&self, user: Option<&str>) -> Vec<JobInfo>;
    fn get_partitions(&self) -> Vec<String>;
    fn get_job_details(&self, job_id: &str) -> Option<serde_json::Value>;
    fn cancel_job(&self, job_id: &str) -> bool;
    fn hold_job(&self, job_id: &str) -> bool;
    fn release_job(&self, job_id: &str) -> bool;
    fn submit_job(
        &self,
        script_path: &str,
        params: &HashMap<String, String>,
    ) -> Result<SubmitOutcome, String>;
    /// `scontrol requeue <id>`: put a running/finished batch job back in the queue.
    fn requeue_job(&self, job_id: &str) -> bool;
    /// `scancel --signal=<sig> [--batch] <id>`. `sig` must be in [`JOB_SIGNALS`].
    fn signal_job(&self, job_id: &str, signal: &str, batch_only: bool) -> bool;
    fn get_sinfo(&self) -> Vec<SinfoRow>;
    fn get_node_info(&self) -> Vec<NodeInfoRow>;
    fn get_sacct(&self, user: Option<&str>, start_time: Option<&str>) -> Vec<SacctRow>;
    fn get_sstat(&self, job_id: &str) -> Option<SstatResult>;
    fn get_storage(&self) -> Vec<StorageInfo>;
    fn get_gpu_utilization(&self, node: Option<&str>) -> Vec<f64>;
    /// Drain the most recent command error, if any, so the UI can surface it.
    /// The mock never errors; only the real controller overrides this.
    fn take_last_error(&self) -> Option<String> {
        None
    }
    /// Whether an interactive srun terminal handover is possible.
    /// The mock (demo mode) has no cluster to hand the terminal to.
    fn supports_interactive(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// RealSlurmController
// ---------------------------------------------------------------------------

/// Exit code reported when a command exceeds its deadline (same convention
/// as timeout(1)).
const RC_TIMEOUT: i32 = 124;

pub struct RealSlurmController {
    timeout_secs: u64,
    gpu_command: Vec<String>,
    // &self methods on a Send trait object need interior mutability here
    last_error: Mutex<Option<String>>,
}

fn spawn_pipe_reader<R: std::io::Read + Send + 'static>(
    mut r: R,
) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = r.read_to_end(&mut buf);
        String::from_utf8_lossy(&buf).to_string()
    })
}

impl RealSlurmController {
    pub fn new(timeout_secs: f64) -> Self {
        Self {
            timeout_secs: (timeout_secs.max(1.0)) as u64,
            gpu_command: vec!["nvidia-smi".to_string()],
            last_error: Mutex::new(None),
        }
    }

    /// Override the GPU sampling command (config `[gpu] command`), given as
    /// a whitespace-separated command line.
    pub fn with_gpu_command(mut self, command: &str) -> Self {
        let argv: Vec<String> = command.split_whitespace().map(str::to_string).collect();
        if !argv.is_empty() {
            self.gpu_command = argv;
        }
        self
    }

    fn record_error(&self, msg: String) {
        if let Ok(mut guard) = self.last_error.lock() {
            *guard = Some(msg);
        }
    }

    /// Record a non-zero exit, with a dedicated hint when the failure looks
    /// like a Slurm build without --json support (pre-20.11 / no serializer).
    fn record_cmd_failure(&self, cmd: &[&str], rc: i32, stderr: &str) {
        let name = cmd.first().copied().unwrap_or("command");
        let lower = stderr.to_lowercase();
        let msg = if cmd.contains(&"--json")
            && (lower.contains("unrecognized option")
                || lower.contains("invalid option")
                || lower.contains("unrecognized argument"))
        {
            format!("{name} --json unsupported — Slurm >= 20.11 with JSON support required")
        } else if rc == RC_TIMEOUT {
            stderr.trim().to_string()
        } else {
            format!("{name} failed (rc={rc}): {}", stderr.trim())
        };
        self.record_error(msg);
    }

    /// Run a command with a hard deadline so a hung Slurm client (dead
    /// slurmctld, stuck network FS) cannot freeze the single-threaded UI
    /// forever. stdout/stderr are drained on threads — a child writing more
    /// than the pipe buffer (squeue --json easily does) would otherwise
    /// deadlock against the try_wait loop.
    fn run_cmd(&self, cmd: &[&str], timeout_secs: u64) -> (i32, String, String) {
        let mut child = match Command::new(cmd[0])
            .args(&cmd[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return (127, String::new(), format!("{}: command not found", cmd[0]));
            }
            Err(e) => return (126, String::new(), format!("{}: {e}", cmd[0])),
        };

        let out_reader = child.stdout.take().map(spawn_pipe_reader);
        let err_reader = child.stderr.take().map(spawn_pipe_reader);

        enum Outcome {
            Exited(i32),
            TimedOut,
            WaitFailed(String),
        }

        let deadline = Instant::now() + Duration::from_secs(timeout_secs.max(1));
        let outcome = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Outcome::Exited(status.code().unwrap_or(1)),
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        break Outcome::TimedOut;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(e) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Outcome::WaitFailed(e.to_string());
                }
            }
        };

        // The child has exited (or been killed) — the pipes are at EOF, so
        // these joins cannot block.
        let stdout = out_reader.and_then(|h| h.join().ok()).unwrap_or_default();
        let stderr = err_reader.and_then(|h| h.join().ok()).unwrap_or_default();

        match outcome {
            Outcome::Exited(rc) => (rc, stdout, stderr),
            Outcome::TimedOut => (
                RC_TIMEOUT,
                stdout,
                format!("{} timed out after {}s", cmd[0], timeout_secs.max(1)),
            ),
            Outcome::WaitFailed(e) => (1, stdout, format!("{}: {e}", cmd[0])),
        }
    }

    /// Run a command at the configured timeout; on failure record the error
    /// for the status bar and return None.
    fn run_checked(&self, cmd: &[&str]) -> Option<String> {
        let (rc, stdout, stderr) = self.run_cmd(cmd, self.timeout_secs);
        if rc != 0 {
            self.record_cmd_failure(cmd, rc, &stderr);
            return None;
        }
        Some(stdout)
    }
}

impl SlurmController for RealSlurmController {
    fn current_user(&self) -> String {
        std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "unknown".to_string())
    }

    fn get_cluster_name(&self) -> String {
        let Some(stdout) = self.run_checked(&["scontrol", "show", "config"]) else {
            return "unknown".to_string();
        };
        for line in stdout.lines() {
            if let Some((key, val)) = line.split_once('=') {
                if key.trim() == "ClusterName" {
                    return val.trim().to_string();
                }
            }
        }
        "unknown".to_string()
    }

    fn get_queue(&self, user: Option<&str>) -> Vec<JobInfo> {
        let user = user.map(|s| s.to_string())
            .unwrap_or_else(|| self.current_user());
        if validate_safe_filter(&user).is_err() {
            self.record_error(format!("Invalid user name: {user:?}"));
            return Vec::new();
        }
        let Some(stdout) = self.run_checked(&["squeue", "-u", &user, "--json"]) else {
            return Vec::new();
        };
        let data: serde_json::Value = match serde_json::from_str(&stdout) {
            Ok(v) => v,
            Err(_) => {
                self.record_error(
                    "squeue --json returned non-JSON output — Slurm >= 20.11 with JSON support required".into(),
                );
                return Vec::new();
            }
        };
        parse_squeue_json(&data, &user, now_epoch())
    }

    fn get_partitions(&self) -> Vec<String> {
        let Some(stdout) = self.run_checked(&["sinfo", "-h", "-o", "%P"]) else {
            return Vec::new();
        };
        stdout
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.trim().trim_end_matches('*').to_string())
            .collect()
    }

    fn get_job_details(&self, job_id: &str) -> Option<serde_json::Value> {
        let job_id = validate_job_id(job_id).ok()?;
        let cmd = ["scontrol", "show", "job", job_id.as_str(), "--json"];
        let (rc, stdout, stderr) = self.run_cmd(&cmd, self.timeout_secs);
        if rc == 0 {
            if let Some(job) = serde_json::from_str::<serde_json::Value>(&stdout)
                .ok()
                .as_ref()
                .and_then(first_job)
            {
                return Some(job);
            }
        } else if !stderr.to_lowercase().contains("invalid job id") {
            self.record_cmd_failure(&cmd, rc, &stderr);
            return None;
        }
        // slurmctld forgets finished jobs after MinJobAge (default 300s);
        // the accounting database still has them.
        let stdout = self.run_checked(&["sacct", "-j", &job_id, "--json"])?;
        let data: serde_json::Value = serde_json::from_str(&stdout).ok()?;
        let jobs = data.get("jobs")?.as_array()?;
        let job = jobs
            .iter()
            .find(|j| j.get("job_id").map(slurm_val_to_string).as_deref() == Some(job_id.as_str()))
            .or_else(|| jobs.first())?;
        Some(sacct_job_to_details(job))
    }

    fn cancel_job(&self, job_id: &str) -> bool {
        let Ok(job_id) = validate_job_id(job_id) else {
            return false;
        };
        self.run_checked(&["scancel", &job_id]).is_some()
    }

    fn hold_job(&self, job_id: &str) -> bool {
        let Ok(job_id) = validate_job_id(job_id) else {
            return false;
        };
        self.run_checked(&["scontrol", "hold", &job_id]).is_some()
    }

    fn release_job(&self, job_id: &str) -> bool {
        let Ok(job_id) = validate_job_id(job_id) else {
            return false;
        };
        self.run_checked(&["scontrol", "release", &job_id]).is_some()
    }

    fn requeue_job(&self, job_id: &str) -> bool {
        let Ok(job_id) = validate_job_id(job_id) else {
            return false;
        };
        self.run_checked(&["scontrol", "requeue", &job_id]).is_some()
    }

    fn signal_job(&self, job_id: &str, signal: &str, batch_only: bool) -> bool {
        let Ok(job_id) = validate_job_id(job_id) else {
            return false;
        };
        if !JOB_SIGNALS.contains(&signal) {
            self.record_error(format!("Unsupported signal: {signal:?}"));
            return false;
        }
        let sig = format!("--signal={signal}");
        let mut cmd = vec!["scancel", sig.as_str()];
        if batch_only {
            cmd.push("--batch");
        }
        cmd.push(&job_id);
        self.run_checked(&cmd).is_some()
    }

    fn submit_job(
        &self,
        script_path: &str,
        params: &HashMap<String, String>,
    ) -> Result<SubmitOutcome, String> {
        // --parsable: "jobid[;cluster]" on stdout, the documented stable
        // format (the human "Submitted batch job N" text may be localised
        // or wrapped by site plugins).
        let mut args: Vec<String> = vec!["sbatch".to_string(), "--parsable".to_string()];
        for (key, value) in params {
            validate_safe_key(key)?;
            validate_param_value(value)?;
            if value.is_empty() {
                args.push(format!("--{key}"));
            } else {
                args.push(format!("--{key}={value}"));
            }
        }
        let sp = if script_path.starts_with('-') {
            format!("./{script_path}")
        } else {
            script_path.to_string()
        };
        args.push(sp);

        let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let (rc, stdout, stderr) = self.run_cmd(&refs, self.timeout_secs);
        if rc != 0 {
            return Err(format!("sbatch failed (rc={rc}): {}", stderr.trim()));
        }
        if params.contains_key("test-only") {
            // sbatch --test-only prints its estimate on stderr and queues nothing.
            let msg = stderr.trim().trim_start_matches("sbatch:").trim();
            return Ok(SubmitOutcome::TestOnly(msg.to_string()));
        }
        parse_parsable_job_id(&stdout)
            .map(SubmitOutcome::Submitted)
            .ok_or_else(|| format!("Unexpected sbatch output: {stdout:?}"))
    }

    fn get_sinfo(&self) -> Vec<SinfoRow> {
        let fmt = "%P|%a|%l|%D|%T|%N|%c|%m|%G";
        let Some(stdout) = self.run_checked(&["sinfo", "-h", "-o", fmt]) else {
            return Vec::new();
        };
        let keys = [
            "partition", "avail", "timelimit", "nodes", "state", "nodelist",
            "cpus", "memory", "gres",
        ];
        stdout
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.split('|').collect();
                if parts.len() >= keys.len() {
                    Some(SinfoRow {
                        partition: parts[0].trim().trim_end_matches('*').to_string(),
                        avail: parts[1].trim().to_string(),
                        timelimit: parts[2].trim().to_string(),
                        nodes: parts[3].trim().to_string(),
                        state: parts[4].trim().to_string(),
                        nodelist: parts[5].trim().to_string(),
                        cpus: parts[6].trim().to_string(),
                        memory: parts[7].trim().to_string(),
                        gres: parts[8].trim().to_string(),
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    fn get_node_info(&self) -> Vec<NodeInfoRow> {
        // Prefer --json: the text format allows values with spaces and '='
        // (OS=, Reason=, CfgTRES=) that cannot be tokenized reliably. A
        // failure here is not recorded — the text fallback below may work.
        let (rc, stdout, _) = self.run_cmd(&["scontrol", "show", "nodes", "--json"], self.timeout_secs);
        if rc == 0 {
            if let Ok(data) = serde_json::from_str::<serde_json::Value>(&stdout) {
                if data.get("nodes").is_some() {
                    return parse_scontrol_nodes_json(&data);
                }
            }
        }
        // Fallback for Slurm builds without JSON support
        let Some(stdout) = self.run_checked(&["scontrol", "show", "nodes"]) else {
            return Vec::new();
        };
        parse_scontrol_nodes_text(&stdout)
    }

    fn get_sacct(&self, user: Option<&str>, start_time: Option<&str>) -> Vec<SacctRow> {
        let args = [
            "sacct", "-n", "-P",
            "--format=JobID,JobName,Partition,State,Elapsed,TotalCPU,MaxRSS,ExitCode",
        ];
        let mut owned_args = Vec::new();
        if let Some(u) = user {
            if validate_safe_filter(u).is_err() {
                return Vec::new();
            }
            owned_args.push("-u".to_string());
            owned_args.push(u.to_string());
        }
        if let Some(st) = start_time {
            if validate_safe_filter(st).is_err() {
                return Vec::new();
            }
            owned_args.push("-S".to_string());
            owned_args.push(st.to_string());
        }
        let mut full_args: Vec<&str> = args.to_vec();
        for a in &owned_args {
            full_args.push(a.as_str());
        }
        let Some(stdout) = self.run_checked(&full_args) else {
            return Vec::new();
        };
        parse_sacct_output(&stdout)
    }

    fn get_sstat(&self, job_id: &str) -> Option<SstatResult> {
        let Ok(job_id) = validate_job_id(job_id) else {
            return None;
        };
        // No error recording: sstat legitimately fails for jobs that are not
        // running (or not ours) — surfacing that every poll would be noise.
        for suffix in &["", ".batch"] {
            let jid = format!("{job_id}{suffix}");
            let (rc, stdout, _) = self.run_cmd(
                &["sstat", "-n", "-P", "--format=AveCPU,MaxRSS,MaxVMSize", "-j", &jid],
                self.timeout_secs,
            );
            if rc != 0 {
                continue;
            }
            for line in stdout.lines() {
                let parts: Vec<&str> = line.split('|').collect();
                if parts.len() >= 3 && parts[..3].iter().any(|p| !p.trim().is_empty()) {
                    return Some(SstatResult {
                        avg_cpu: parts[0].trim().to_string(),
                        max_rss: parts[1].trim().to_string(),
                        max_vmsize: parts[2].trim().to_string(),
                    });
                }
            }
        }
        None
    }

    fn get_storage(&self) -> Vec<StorageInfo> {
        // -P (POSIX) keeps each filesystem on a single line; plain `df -h`
        // wraps long device names, which breaks the positional parse below.
        let (rc, stdout, _) = self.run_cmd(&["df", "-hP"], self.timeout_secs.min(10));
        if rc != 0 {
            return Vec::new();
        }
        stdout
            .lines()
            .skip(1) // skip header
            .filter_map(|line| {
                let cols: Vec<&str> = line.split_whitespace().collect();
                if cols.len() >= 6 {
                    Some(StorageInfo {
                        filesystem: cols[0].to_string(),
                        size: cols[1].to_string(),
                        used: cols[2].to_string(),
                        avail: cols[3].to_string(),
                        use_pct: cols[4].to_string(),
                        mount: cols[5..].join(" "),
                    })
                } else {
                    None
                }
            })
            .collect()
    }

    fn get_gpu_utilization(&self, node: Option<&str>) -> Vec<f64> {
        let mut base: Vec<&str> = self.gpu_command.iter().map(|s| s.as_str()).collect();
        // A bare nvidia-smi gets the standard utilization query appended; a
        // custom command is expected to print one utilization value per line.
        if base.len() == 1 && base[0].ends_with("nvidia-smi") {
            base.extend_from_slice(&[
                "--query-gpu=utilization.gpu",
                "--format=csv,noheader,nounits",
            ]);
        }
        // Short deadline: this may ssh to a compute node from the UI thread,
        // and a dead node must stall at most one poll tick.
        let timeout = self.timeout_secs.min(10);
        let (rc, stdout, stderr) = if let Some(n) = node {
            let mut cmd = vec!["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=2", n];
            cmd.extend_from_slice(&base);
            self.run_cmd(&cmd, timeout)
        } else {
            self.run_cmd(&base, timeout)
        };
        if rc != 0 {
            self.record_error(format!(
                "GPU monitor failed (rc={rc}): {}",
                stderr.trim()
            ));
            return Vec::new();
        }
        stdout
            .lines()
            .filter_map(|l| l.trim().parse::<f64>().ok())
            .collect()
    }

    fn take_last_error(&self) -> Option<String> {
        self.last_error.lock().ok().and_then(|mut g| g.take())
    }
}

// ---------------------------------------------------------------------------
// Shared JSON helpers
// ---------------------------------------------------------------------------

/// Convert a Slurm JSON value to a displayable string.
///
/// Slurm 22+ encodes many scalar fields as `{"number": N, "set": bool, "infinite": bool}`.
/// Arrays (e.g. `job_state`) are joined with commas.
pub fn slurm_val_to_string(val: &serde_json::Value) -> String {
    match val {
        serde_json::Value::String(s) => {
            if s.is_empty() || s == "(null)" { String::new() } else { s.clone() }
        }
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(", "),
        serde_json::Value::Object(map) => {
            // `{"infinite": true, ...}` → "UNLIMITED"
            if map.get("infinite").and_then(|v| v.as_bool()).unwrap_or(false) {
                return "UNLIMITED".to_string();
            }
            // `{"set": false, ...}` → the field has no value (NO_VAL)
            if map.get("set").and_then(|v| v.as_bool()) == Some(false) {
                return String::new();
            }
            // `{"number": N, ...}` → "N"
            if let Some(n) = map.get("number") {
                return slurm_val_to_string(n);
            }
            // Fallback: serialize the whole object
            val.to_string()
        }
        serde_json::Value::Null => String::new(),
    }
}

/// Numeric value of a Slurm JSON field, unwrapping the modern
/// `{"number": N, "set": b, "infinite": b}` envelope as well as plain numbers
/// and numeric strings. Returns None when the field is absent or non-numeric.
pub fn slurm_val_to_f64(val: &serde_json::Value) -> Option<f64> {
    match val {
        serde_json::Value::Object(map) => {
            if map.get("set").and_then(|v| v.as_bool()) == Some(false)
                || map.get("infinite").and_then(|v| v.as_bool()) == Some(true)
            {
                return None;
            }
            map.get("number").and_then(slurm_val_to_f64)
        }
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

/// First hostname of a Slurm node list expression:
/// "gpu[001-008,010],cpu01" -> "gpu001", "node01,node02" -> "node01".
pub fn first_node_of(node_list: &str) -> Option<String> {
    let s = node_list.trim();
    if s.is_empty() || s == "(null)" {
        return None;
    }
    if let Some(idx) = s.find('[') {
        let prefix = &s[..idx];
        let first = s[idx + 1..]
            .split(['-', ',', ']'])
            .next()
            .unwrap_or("");
        if first.is_empty() {
            return None;
        }
        return Some(format!("{prefix}{first}"));
    }
    Some(s.split(',').next().unwrap_or(s).to_string())
}

/// Extract the base job state from a squeue/scontrol JSON entry.
///
/// Modern Slurm emits `"job_state": ["RUNNING"]` (an array of state flags);
/// older versions emit a plain string. Returns the base state, or "UNKNOWN".
pub fn job_state_of(entry: &serde_json::Value) -> String {
    match entry.get("job_state") {
        Some(serde_json::Value::Array(arr)) => arr
            .first()
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN")
            .to_string(),
        Some(serde_json::Value::String(s)) => s.clone(),
        _ => "UNKNOWN".to_string(),
    }
}

/// First value among `keys` that is present and not JSON null. Slurm has
/// renamed several fields across data_parser versions; callers list the
/// modern name first and older names after it.
pub fn field<'a>(v: &'a serde_json::Value, keys: &[&str]) -> Option<&'a serde_json::Value> {
    keys.iter()
        .filter_map(|k| v.get(*k))
        .find(|x| !x.is_null())
}

/// Integer value of a Slurm JSON number, treating an unset envelope
/// (`{"set": false}`), zero-or-negative, and non-numeric values as None.
fn positive_i64(v: Option<&serde_json::Value>) -> Option<i64> {
    v.and_then(slurm_val_to_f64)
        .map(|n| n as i64)
        .filter(|&n| n > 0)
}

/// Current Unix time in seconds.
pub fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Elapsed run time of a job in seconds, computed the way squeue's `%M`
/// does from the fields `squeue --json` / `scontrol show job --json` expose
/// (they carry `start_time`, `suspend_time`, `pre_sus_time`, not an elapsed
/// counter). The sacct layout (`time.elapsed`) and a legacy `run_time` are
/// honoured when present. Pending jobs report 0.
pub fn job_elapsed_secs(entry: &serde_json::Value, now: i64) -> i64 {
    if let Some(e) = entry.get("time").and_then(|t| t.get("elapsed")) {
        if let Some(n) = slurm_val_to_f64(e) {
            return n.max(0.0) as i64;
        }
    }
    let state = job_state_of(entry);
    let start = positive_i64(entry.get("start_time"));
    let pre_sus = positive_i64(entry.get("pre_sus_time")).unwrap_or(0);
    let suspend = positive_i64(entry.get("suspend_time"));
    let elapsed = match state.as_str() {
        "PENDING" | "CONFIGURING" if start.is_none_or(|s| s > now) => Some(0),
        "SUSPENDED" => Some(pre_sus),
        "RUNNING" | "COMPLETING" | "STOPPED" | "SIGNALING" | "STAGE_OUT" | "RESIZING" => {
            match (suspend, start) {
                (Some(su), _) => Some(pre_sus + (now - su)),
                (None, Some(st)) => Some(now - st),
                _ => None,
            }
        }
        _ => match (start, positive_i64(entry.get("end_time"))) {
            (Some(st), Some(en)) if en >= st => Some(en - st),
            _ => None,
        },
    };
    elapsed
        .or_else(|| positive_i64(entry.get("run_time")))
        .unwrap_or(0)
        .max(0)
}

/// Format a Unix timestamp as local `YYYY-MM-DDTHH:MM:SS` (Slurm's own
/// display format). Returns an empty string for unset or zero values.
pub fn format_epoch(val: Option<&serde_json::Value>) -> String {
    let Some(ts) = positive_i64(val) else {
        return String::new();
    };
    #[cfg(unix)]
    {
        let t: libc::time_t = ts as libc::time_t;
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        // SAFETY: localtime_r only writes into the provided tm struct.
        if !unsafe { libc::localtime_r(&t, &mut tm) }.is_null() {
            return format!(
                "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
                tm.tm_year + 1900,
                tm.tm_mon + 1,
                tm.tm_mday,
                tm.tm_hour,
                tm.tm_min,
                tm.tm_sec
            );
        }
    }
    ts.to_string()
}

/// The first job object of a `{"jobs": [...]}` response.
fn first_job(data: &serde_json::Value) -> Option<serde_json::Value> {
    data.get("jobs")
        .and_then(|j| j.as_array())
        .and_then(|arr| arr.first().cloned())
}

/// Job ID from `sbatch --parsable` output: `jobid` or `jobid;cluster`.
pub fn parse_parsable_job_id(stdout: &str) -> Option<String> {
    let line = stdout.lines().map(str::trim).rfind(|l| !l.is_empty())?;
    let id = line.split(';').next()?.trim();
    JOB_ID_RE.is_match(id).then(|| id.to_string())
}

// ---------------------------------------------------------------------------
// Parse squeue JSON job entry
// ---------------------------------------------------------------------------

/// Parse a whole `squeue --json` document, keeping only `user`'s jobs.
/// Slurm documents that `--json` may ignore the `-u` filter on some
/// versions, so filtering is repeated here.
pub fn parse_squeue_json(data: &serde_json::Value, user: &str, now: i64) -> Vec<JobInfo> {
    let Some(jobs) = data.get("jobs").and_then(|j| j.as_array()) else {
        return Vec::new();
    };
    jobs.iter()
        .map(|e| parse_job_entry(e, now))
        .filter(|j| j.user.is_empty() || j.user == user)
        .collect()
}

pub fn parse_job_entry(entry: &serde_json::Value, now: i64) -> JobInfo {
    let state = job_state_of(entry);

    // Every scalar field goes through slurm_val_to_string, which unwraps the
    // modern {"number":N,"set":b,"infinite":b} envelope and plain values alike.
    let get = |keys: &[&str]| -> String {
        field(entry, keys).map(slurm_val_to_string).unwrap_or_default()
    };

    let time_used = match entry.get("time") {
        // Pre-formatted string from some wrappers / legacy output
        Some(serde_json::Value::String(s)) => s.clone(),
        _ => format_slurm_duration(job_elapsed_secs(entry, now)),
    };

    let node_count = get(&["node_count"]);
    let node_list = get(&["nodes"]);

    let array_job_id = get(&["array_job_id"]);
    let array_job_id = if array_job_id == "0" { String::new() } else { array_job_id };
    let (array_task_id, array_task_string) = if array_job_id.is_empty() {
        (String::new(), String::new())
    } else {
        (get(&["array_task_id"]), get(&["array_task_string"]))
    };

    JobInfo {
        job_id: get(&["job_id"]),
        name: get(&["name"]),
        partition: get(&["partition"]),
        state,
        time_used,
        nodes: if node_count.is_empty() {
            node_list.clone()
        } else {
            node_count
        },
        reason: get(&["state_reason"]),
        user: get(&["user_name", "user"]),
        work_dir: get(&["current_working_directory", "working_directory"]),
        stdout_path: get(&["stdout_expanded", "standard_output"]),
        stderr_path: get(&["stderr_expanded", "standard_error"]),
        submit_time: format_epoch(entry.get("submit_time")),
        node_list,
        array_job_id,
        array_task_id,
        array_task_string,
        extra: HashMap::new(),
    }
}

/// Path of a job's stdout (or stderr) file as a local path to open.
///
/// Prefers the server-side expansion Slurm 24.05+ provides
/// (`stdout_expanded`); otherwise expands the sbatch filename patterns in
/// `standard_output` itself. Relative paths are resolved against the job's
/// working directory, and an unset stdout falls back to sbatch's default
/// `slurm-%j.out` (`slurm-%A_%a.out` for arrays). An unset stderr means
/// stderr goes to the stdout file (sbatch(1), --error).
pub fn resolve_log_path(details: &serde_json::Value, stderr: bool) -> Option<String> {
    use crate::validators::{expand_slurm_filename, FilenameContext};
    let get = |keys: &[&str]| -> String {
        field(details, keys)
            .map(slurm_val_to_string)
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let (exp_key, raw_key) = if stderr {
        ("stderr_expanded", "standard_error")
    } else {
        ("stdout_expanded", "standard_output")
    };
    let expanded = get(&[exp_key]);
    let raw = get(&[raw_key]);
    let array_job_id = get(&["array_job_id"]);
    let array_job_id = if array_job_id == "0" { String::new() } else { array_job_id };
    let ctx = FilenameContext {
        job_id: get(&["job_id"]),
        array_task_id: if array_job_id.is_empty() { String::new() } else { get(&["array_task_id"]) },
        array_job_id,
        job_name: get(&["name"]),
        user: get(&["user_name", "user"]),
        first_node: first_node_of(&get(&["batch_host", "nodes"])).unwrap_or_default(),
    };
    let path = if !expanded.is_empty() {
        expanded
    } else if !raw.is_empty() {
        expand_slurm_filename(&raw, &ctx)
    } else if stderr {
        return resolve_log_path(details, false);
    } else if details.get("slurmterm_source").is_some() {
        // sacct before 24.05 does not record stdout; guessing would mislead.
        return None;
    } else {
        let default = if ctx.array_job_id.is_empty() { "slurm-%j.out" } else { "slurm-%A_%a.out" };
        expand_slurm_filename(default, &ctx)
    };
    if path.starts_with('/') {
        return Some(path);
    }
    let cwd = get(&["current_working_directory", "working_directory"]);
    if cwd.is_empty() {
        Some(path)
    } else {
        Some(format!("{}/{path}", cwd.trim_end_matches('/')))
    }
}

// ---------------------------------------------------------------------------
// sacct JSON → job details
// ---------------------------------------------------------------------------

/// Map one job object of `sacct --json` (the accounting layout) onto the
/// keys `scontrol show job --json` uses, so the Inspector and
/// [`extract_form_state`] can treat both sources alike. The result carries
/// `"slurmterm_source": "sacct"`.
pub fn sacct_job_to_details(job: &serde_json::Value) -> serde_json::Value {
    use serde_json::{json, Map, Value};
    let mut out = Map::new();
    let mut put = |k: &str, v: Option<&Value>| {
        if let Some(v) = v.filter(|v| !v.is_null()) {
            out.insert(k.to_string(), v.clone());
        }
    };
    let time = job.get("time");
    let tget = |k: &str| time.and_then(|t| t.get(k));
    let state = job.get("state");
    let array = job.get("array");
    let required = job.get("required");

    for k in ["job_id", "name", "partition", "account", "qos", "nodes", "comment"] {
        put(k, job.get(k));
    }
    put("user_name", job.get("user"));
    put("job_state", state.and_then(|s| s.get("current")).or(state));
    put("state_reason", state.and_then(|s| s.get("reason")));
    put("time_limit", tget("limit"));
    put("run_time", tget("elapsed"));
    put("start_time", tget("start"));
    put("end_time", tget("end"));
    put("submit_time", tget("submission"));
    put("current_working_directory", job.get("working_directory"));
    put("standard_output", job.get("stdout"));
    put("standard_error", job.get("stderr"));
    put("stdout_expanded", job.get("stdout_expanded"));
    put("stderr_expanded", job.get("stderr_expanded"));
    put("memory_per_node", required.and_then(|r| r.get("memory_per_node")));
    put("memory_per_cpu", required.and_then(|r| r.get("memory_per_cpu")));
    put("array_job_id", array.and_then(|a| a.get("job_id")));
    put("array_task_id", array.and_then(|a| a.get("task_id")));
    put(
        "exit_code",
        job.get("exit_code").and_then(|e| e.get("return_code")).or(job.get("exit_code")),
    );
    put("command", job.get("submit_line").or(job.get("script")));
    out.insert("slurmterm_source".into(), json!("sacct"));
    Value::Object(out)
}

// ---------------------------------------------------------------------------
// sacct parsing
// ---------------------------------------------------------------------------

/// Parse `sacct -n -P --format=JobID,JobName,Partition,State,Elapsed,TotalCPU,MaxRSS,ExitCode`
/// output. Step rows (`123.batch`, `123.0`, …) are folded into their parent
/// job row via [`merge_sacct_steps`].
pub fn parse_sacct_output(stdout: &str) -> Vec<SacctRow> {
    let rows: Vec<SacctRow> = stdout
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('|').collect();
            if parts.len() >= 8 {
                Some(SacctRow {
                    job_id: parts[0].trim().to_string(),
                    name: parts[1].trim().to_string(),
                    partition: parts[2].trim().to_string(),
                    state: parts[3].trim().to_string(),
                    elapsed: parts[4].trim().to_string(),
                    total_cpu: parts[5].trim().to_string(),
                    max_rss: parts[6].trim().to_string(),
                    exit_code: parts[7].trim().to_string(),
                })
            } else {
                None
            }
        })
        .collect();
    merge_sacct_steps(rows)
}

/// Fold sacct step rows into their parent job rows.
///
/// sacct only populates MaxRSS (and often TotalCPU) on step rows like
/// `123.batch` — the job-level row leaves them blank. Keep one row per job,
/// carrying the maximum MaxRSS across its steps and the step TotalCPU when
/// the job row has none.
pub fn merge_sacct_steps(rows: Vec<SacctRow>) -> Vec<SacctRow> {
    use crate::validators::parse_rss_kb;

    let mut jobs: Vec<SacctRow> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut steps: Vec<SacctRow> = Vec::new();

    for row in rows {
        if row.job_id.contains('.') {
            steps.push(row);
        } else {
            index.insert(row.job_id.clone(), jobs.len());
            jobs.push(row);
        }
    }

    for step in steps {
        let parent_id = step.job_id.split('.').next().unwrap_or("");
        let Some(&i) = index.get(parent_id) else {
            continue;
        };
        let job = &mut jobs[i];
        let step_rss = parse_rss_kb(&step.max_rss);
        if step_rss.is_some() && step_rss > parse_rss_kb(&job.max_rss) {
            job.max_rss = step.max_rss;
        }
        if job.total_cpu.trim().is_empty() && !step.total_cpu.trim().is_empty() {
            job.total_cpu = step.total_cpu;
        }
    }

    jobs
}

// ---------------------------------------------------------------------------
// scontrol node parsing
// ---------------------------------------------------------------------------

/// Parse `scontrol show nodes --json`, mapping JSON keys onto the text-format
/// field names the Hardware tab reads (NodeName, State, CPUTot, RealMemory, …).
pub fn parse_scontrol_nodes_json(data: &serde_json::Value) -> Vec<NodeInfoRow> {
    let Some(arr) = data.get("nodes").and_then(|n| n.as_array()) else {
        return Vec::new();
    };
    arr.iter()
        .map(|node| {
            let mut fields = HashMap::new();
            // State is an array of base state + flags: ["IDLE", "DRAIN"].
            // Join with '+' to match the text format ("IDLE+DRAIN").
            if let Some(state) = node.get("state") {
                let s = match state {
                    serde_json::Value::Array(a) => a
                        .iter()
                        .filter_map(|v| v.as_str())
                        .collect::<Vec<_>>()
                        .join("+"),
                    _ => slurm_val_to_string(state),
                };
                if !s.is_empty() {
                    fields.insert("State".to_string(), s);
                }
            }
            // The REST/JSON node schema reports cpu_load as load x 100 in an
            // integer; the text format prints the decimal (CPULoad=12.34).
            if let Some(v) = node.get("cpu_load") {
                let s = match v {
                    serde_json::Value::Number(n) if n.is_i64() || n.is_u64() => {
                        format!("{:.2}", n.as_f64().unwrap_or(0.0) / 100.0)
                    }
                    _ => slurm_val_to_string(v),
                };
                if !s.is_empty() {
                    fields.insert("CPULoad".to_string(), s);
                }
            }
            for (field, json_key) in [
                ("NodeName", "name"),
                ("CPUTot", "cpus"),
                ("CPUAlloc", "alloc_cpus"),
                ("RealMemory", "real_memory"),
                ("AllocMem", "alloc_memory"),
                ("FreeMem", "free_mem"),
                ("Gres", "gres"),
                ("GresUsed", "gres_used"),
                ("Partitions", "partitions"),
                ("Reason", "reason"),
            ] {
                if let Some(v) = node.get(json_key) {
                    let s = slurm_val_to_string(v);
                    if !s.is_empty() {
                        fields.insert(field.to_string(), s);
                    }
                }
            }
            NodeInfoRow { fields }
        })
        .collect()
}

/// A `Key=` token starts a new field in scontrol text output. Real scontrol
/// keys are CamelCase (NodeName, CfgTRES, MCS_label); embedded `foo=bar`
/// fragments inside values (e.g. in Reason=) start lowercase or non-alpha.
fn is_scontrol_key(key: &str) -> bool {
    let mut chars = key.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_uppercase())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Parse plain `scontrol show nodes` text output (fallback for Slurm builds
/// without --json). Splits on whitespace; a `Key=value` token starts a field
/// (split on the FIRST '=' so `CfgTRES=cpu=16,mem=64G` survives) and bare
/// tokens continue the previous value (`OS=Linux 5.14`, multi-word Reason=).
pub fn parse_scontrol_nodes_text(stdout: &str) -> Vec<NodeInfoRow> {
    let mut nodes = Vec::new();
    let mut current: HashMap<String, String> = HashMap::new();
    let mut last_key: Option<String> = None;

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            last_key = None;
            if !current.is_empty() {
                nodes.push(NodeInfoRow {
                    fields: std::mem::take(&mut current),
                });
            }
            continue;
        }
        for token in line.split_whitespace() {
            match token.split_once('=') {
                Some((key, val)) if is_scontrol_key(key) => {
                    current.insert(key.to_string(), val.to_string());
                    last_key = Some(key.to_string());
                }
                _ => {
                    if let Some(ref k) = last_key {
                        if let Some(entry) = current.get_mut(k) {
                            if !entry.is_empty() {
                                entry.push(' ');
                            }
                            entry.push_str(token);
                        }
                    }
                }
            }
        }
    }
    if !current.is_empty() {
        nodes.push(NodeInfoRow { fields: current });
    }
    nodes
}

/// Per-node GPU count from a TRES/GRES request string. Accepts
/// `gres/gpu:2`, `gres:gpu:2`, `gpu:a100:2`, `gres/gpu:a100=2` and the
/// running-job `gres_detail` form `gpu:a100:2(IDX:0-1)`. Returns the value
/// the Composer's GPU field expects ("2" or "a100:2").
fn gpus_from_tres(spec: &str) -> Option<String> {
    for part in spec.split(',') {
        let part = part.trim().split('(').next().unwrap_or("");
        let rest = part
            .strip_prefix("gres/")
            .or_else(|| part.strip_prefix("gres:"))
            .unwrap_or(part);
        let Some(rest) = rest.strip_prefix("gpu") else {
            continue;
        };
        let rest = rest.trim_start_matches([':', '=']);
        // "a100:2" / "a100=2" / "2"
        let norm = rest.replace('=', ":");
        let mut bits: Vec<&str> = norm.split(':').filter(|b| !b.is_empty()).collect();
        let count = bits.pop()?;
        if count.parse::<u32>().is_err() {
            return None;
        }
        return Some(match bits.first() {
            Some(t) => format!("{t}:{count}"),
            None => count.to_string(),
        });
    }
    None
}

/// Extract form state from job details JSON (scontrol or the sacct mapping
/// from [`sacct_job_to_details`]) for resubmission.
pub fn extract_form_state(details: &serde_json::Value) -> HashMap<String, String> {
    let mut state = HashMap::new();
    state.insert("mode".into(), "sbatch".into());

    let get = |keys: &[&str]| -> String {
        field(details, keys).map(slurm_val_to_string).unwrap_or_default()
    };

    state.insert("name".into(), get(&["name"]));
    state.insert("partition".into(), get(&["partition"]));

    // Slurm reports time_limit in minutes; the form wants HH:MM:SS.
    let tl = get(&["time_limit"]);
    if tl == "UNLIMITED" {
        state.insert("time".into(), tl);
    } else if let Ok(min) = tl.parse::<i64>() {
        if min > 0 {
            state.insert("time".into(), format_hms(min * 60));
        }
    }

    let nodes = get(&["node_count"]);
    state.insert(
        "nodes".into(),
        if nodes.is_empty() { "1".into() } else { nodes },
    );
    state.insert("ntasks".into(), get(&["tasks_per_node"]));
    state.insert("cpus".into(), get(&["cpus_per_task"]));

    // Memory (MiB). Per-node and per-CPU requests are distinct sbatch
    // options (--mem / --mem-per-cpu) and must not be confused.
    let mib_to_opt = |mb: f64| -> String {
        if mb >= 1024.0 && (mb / 1024.0).fract() == 0.0 {
            format!("{:.0}G", mb / 1024.0)
        } else {
            format!("{mb:.0}M")
        }
    };
    let per_node = field(details, &["memory_per_node", "minimum_memory_per_node"])
        .and_then(slurm_val_to_f64)
        .filter(|&m| m > 0.0);
    let per_cpu = field(details, &["memory_per_cpu"])
        .and_then(slurm_val_to_f64)
        .filter(|&m| m > 0.0);
    if let Some(mb) = per_node {
        state.insert("memory".into(), mib_to_opt(mb));
    } else if let Some(mb) = per_cpu {
        state.insert("extra.mem-per-cpu".into(), mib_to_opt(mb));
    }

    // The GPUs form field holds "count" or "type:count"; build_params
    // re-adds the "gpu:" prefix. tres_per_node is what was requested (keep
    // the type if one was asked for). gres_detail (running jobs only; one
    // entry per node) is the allocation: its type is whatever the node had,
    // not necessarily a requirement, so only its count is reused.
    let gpus = gpus_from_tres(&get(&["tres_per_node"])).or_else(|| {
        gpus_from_tres(&get(&["gres_detail"]))
            .map(|g| g.rsplit(':').next().unwrap_or(&g).to_string())
    });
    if let Some(g) = gpus {
        state.insert("gpus".into(), g);
    }

    // sbatch options that have no dedicated form field
    for (opt, keys) in [
        ("account", &["account"][..]),
        ("qos", &["qos"][..]),
        ("dependency", &["dependency"][..]),
        ("constraint", &["features"][..]),
        ("reservation", &["reservation"][..]),
        ("comment", &["comment"][..]),
    ] {
        let v = get(keys);
        if !v.is_empty() && v != "(null)" {
            state.insert(format!("extra.{opt}"), v);
        }
    }
    let ntasks = get(&["tasks"]);
    if state.get("ntasks").is_none_or(|v| v.is_empty()) && !ntasks.is_empty() {
        state.insert("extra.ntasks".into(), ntasks);
    }

    state.insert("script".into(), get(&["command"]));
    state.insert("output".into(), get(&["standard_output"]));
    state.insert("error".into(), get(&["standard_error"]));

    state
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_cmd_kills_command_at_deadline() {
        let ctl = RealSlurmController::new(30.0);
        let start = Instant::now();
        let (rc, _, stderr) = ctl.run_cmd(&["sleep", "5"], 1);
        assert_eq!(rc, RC_TIMEOUT);
        assert!(stderr.contains("timed out"));
        assert!(start.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn run_cmd_captures_output_on_success() {
        let ctl = RealSlurmController::new(30.0);
        let (rc, stdout, stderr) = ctl.run_cmd(&["sh", "-c", "echo hi; echo err >&2"], 5);
        assert_eq!(rc, 0);
        assert_eq!(stdout.trim(), "hi");
        assert_eq!(stderr.trim(), "err");
    }

    #[test]
    fn run_cmd_reports_missing_command() {
        let ctl = RealSlurmController::new(30.0);
        let (rc, _, stderr) = ctl.run_cmd(&["slurm-term-definitely-not-a-command"], 5);
        assert_eq!(rc, 127);
        assert!(stderr.contains("command not found"));
    }

    #[test]
    fn take_last_error_drains_once() {
        let ctl = RealSlurmController::new(30.0);
        ctl.record_error("boom".into());
        assert_eq!(ctl.take_last_error(), Some("boom".to_string()));
        assert_eq!(ctl.take_last_error(), None);
    }

    #[test]
    fn record_cmd_failure_detects_missing_json_support() {
        let ctl = RealSlurmController::new(30.0);
        ctl.record_cmd_failure(
            &["squeue", "-u", "me", "--json"],
            1,
            "squeue: unrecognized option '--json'",
        );
        let msg = ctl.take_last_error().unwrap();
        assert!(msg.contains("--json unsupported"), "got: {msg}");
    }

    #[test]
    fn extract_form_state_reads_modern_wrapped_scalars() {
        let details = serde_json::json!({
            "name": "train",
            "partition": "gpu",
            "time_limit": {"set": true, "infinite": false, "number": 90},
            "node_count": {"set": true, "infinite": false, "number": 2},
            "cpus_per_task": {"set": true, "infinite": false, "number": 8},
            "minimum_memory_per_node": {"set": true, "infinite": false, "number": 32768},
            "gres_detail": "gpu:a100:2(IDX:0-1)",
            "command": "/home/me/train.sh",
            "standard_output": "out.log",
        });
        let s = extract_form_state(&details);
        assert_eq!(s["name"], "train");
        assert_eq!(s["time"], "01:30:00"); // 90 minutes
        assert_eq!(s["nodes"], "2");
        assert_eq!(s["cpus"], "8");
        assert_eq!(s["memory"], "32G");
        assert_eq!(s["gpus"], "2"); // bare count; build_params re-adds "gpu:"
        assert_eq!(s["script"], "/home/me/train.sh");
    }

    #[test]
    fn extract_form_state_handles_legacy_and_missing_fields() {
        // Legacy plain scalars, and a "(null)" gres that must not leak through
        let details = serde_json::json!({
            "name": "old",
            "time_limit": 60,
            "cpus_per_task": 4,
            "gres_detail": "(null)",
        });
        let s = extract_form_state(&details);
        assert_eq!(s["time"], "01:00:00");
        assert_eq!(s["cpus"], "4");
        assert_eq!(s["nodes"], "1"); // defaulted
        assert!(!s.contains_key("gpus"));
        assert!(!s.contains_key("memory"));
    }

    #[test]
    fn first_node_of_expands_bracket_ranges() {
        assert_eq!(first_node_of("gpu[001-008]"), Some("gpu001".to_string()));
        assert_eq!(first_node_of("gpu[001-008,010],cpu01"), Some("gpu001".to_string()));
        assert_eq!(first_node_of("node01,node02"), Some("node01".to_string()));
        assert_eq!(first_node_of("node01"), Some("node01".to_string()));
        assert_eq!(first_node_of(""), None);
        assert_eq!(first_node_of("(null)"), None);
    }

    #[test]
    fn job_state_of_handles_array_and_string() {
        let modern: serde_json::Value = serde_json::json!({"job_state": ["RUNNING"]});
        assert_eq!(job_state_of(&modern), "RUNNING");
        let legacy: serde_json::Value = serde_json::json!({"job_state": "PENDING"});
        assert_eq!(job_state_of(&legacy), "PENDING");
        let missing: serde_json::Value = serde_json::json!({});
        assert_eq!(job_state_of(&missing), "UNKNOWN");
        let empty_arr: serde_json::Value = serde_json::json!({"job_state": []});
        assert_eq!(job_state_of(&empty_arr), "UNKNOWN");
    }

    #[test]
    fn sacct_merge_pulls_max_rss_from_batch_step() {
        // Real sacct leaves MaxRSS blank on the job row; only steps carry it.
        let out = "\
123|train|gpu|COMPLETED|01:00:00|00:00:00||0:0
123.batch|batch||COMPLETED|01:00:00|00:59:00|1234567K|0:0
123.extern|extern||COMPLETED|01:00:00|00:00:00|100K|0:0
124|other|cpu|FAILED|00:10:00|00:05:00||1:0
";
        let rows = parse_sacct_output(out);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].job_id, "123");
        assert_eq!(rows[0].max_rss, "1234567K");
        assert_eq!(rows[1].job_id, "124");
        assert_eq!(rows[1].max_rss, "");
    }

    #[test]
    fn sacct_merge_compares_across_units_and_fills_total_cpu() {
        let out = "\
50|job|p|COMPLETED|01:00:00|||0:0
50.batch|batch||COMPLETED|01:00:00|00:30:00|500000K|0:0
50.0|step||COMPLETED|01:00:00|00:20:00|600M|0:0
";
        let rows = parse_sacct_output(out);
        assert_eq!(rows.len(), 1);
        // 600M = 614400K > 500000K
        assert_eq!(rows[0].max_rss, "600M");
        // parent TotalCPU was blank -> filled from first step
        assert_eq!(rows[0].total_cpu, "00:30:00");
    }

    #[test]
    fn sacct_orphan_steps_are_dropped() {
        let out = "99.batch|batch||COMPLETED|01:00:00|00:59:00|1K|0:0\n";
        assert!(parse_sacct_output(out).is_empty());
    }

    #[test]
    fn scontrol_nodes_text_keeps_spaces_and_equals_in_values() {
        let out = "\
NodeName=node001 Arch=x86_64 CoresPerSocket=8
   CPUAlloc=0 CPUTot=16 CPULoad=0.01
   OS=Linux 5.14.0-284.11.1.el9_2.x86_64 #1 SMP
   RealMemory=64000 AllocMem=0 FreeMem=51000 Sockets=2
   State=IDLE+DRAIN ThreadsPerCore=1 Partitions=main,gpu
   CfgTRES=cpu=16,mem=64000M,billing=16
   Reason=Not responding [slurm@2026-01-01T00:00:00]

NodeName=node002 CPUTot=32 State=ALLOCATED RealMemory=128000
";
        let nodes = parse_scontrol_nodes_text(out);
        assert_eq!(nodes.len(), 2);
        let n1 = &nodes[0].fields;
        assert_eq!(n1["NodeName"], "node001");
        assert_eq!(n1["OS"], "Linux 5.14.0-284.11.1.el9_2.x86_64 #1 SMP");
        assert_eq!(n1["CfgTRES"], "cpu=16,mem=64000M,billing=16");
        assert_eq!(n1["Reason"], "Not responding [slurm@2026-01-01T00:00:00]");
        assert_eq!(n1["State"], "IDLE+DRAIN");
        assert_eq!(n1["RealMemory"], "64000");
        let n2 = &nodes[1].fields;
        assert_eq!(n2["NodeName"], "node002");
        assert_eq!(n2["State"], "ALLOCATED");
    }

    #[test]
    fn scontrol_nodes_json_maps_to_text_field_names() {
        let data = serde_json::json!({
            "nodes": [{
                "name": "gpu001",
                "state": ["IDLE", "DRAIN"],
                "cpus": 16,
                "real_memory": 64000,
                "free_mem": {"set": true, "infinite": false, "number": 51000},
                "gres": "gpu:a100:4",
                "partitions": ["main", "gpu"],
                "cpu_load": 0.5
            }]
        });
        let nodes = parse_scontrol_nodes_json(&data);
        assert_eq!(nodes.len(), 1);
        let f = &nodes[0].fields;
        assert_eq!(f["NodeName"], "gpu001");
        assert_eq!(f["State"], "IDLE+DRAIN");
        assert_eq!(f["CPUTot"], "16");
        assert_eq!(f["RealMemory"], "64000");
        assert_eq!(f["FreeMem"], "51000");
        assert_eq!(f["Partitions"], "main, gpu");
        assert_eq!(f["CPULoad"], "0.5");
    }

    #[test]
    fn parse_job_entry_reads_modern_squeue_json() {
        let entry = serde_json::json!({
            "job_id": 4242,
            "name": "train",
            "partition": "gpu",
            "job_state": ["RUNNING"],
            "time": {"elapsed": 3725},
            "node_count": {"set": true, "infinite": false, "number": 2},
            "user_name": "alice",
            "state_reason": "None",
            "nodes": "gpu[001-002]"
        });
        let job = parse_job_entry(&entry, 0);
        assert_eq!(job.job_id, "4242");
        assert_eq!(job.state, "RUNNING");
        assert_eq!(job.time_used, "01:02:05");
        assert_eq!(job.nodes, "2");
        assert_eq!(job.node_list, "gpu[001-002]");
    }

    const NOW: i64 = 1700003725; // 1h02m05s after the fixture start_time

    fn fixture(name: &str) -> serde_json::Value {
        let text = match name {
            "squeue" => include_str!("../tests/fixtures/squeue_23.11.json"),
            "scontrol" => include_str!("../tests/fixtures/scontrol_job_24.05.json"),
            "sacct" => include_str!("../tests/fixtures/sacct_job.json"),
            _ => unreachable!(),
        };
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn squeue_json_computes_elapsed_from_start_time() {
        let jobs = parse_squeue_json(&fixture("squeue"), "alice", NOW);
        let train = jobs.iter().find(|j| j.job_id == "5001").unwrap();
        assert_eq!(train.time_used, "01:02:05");
        assert_eq!(train.work_dir, "/home/alice/run");
        assert_eq!(train.nodes, "2");
        assert_eq!(train.node_list, "gpu[001-002]");
        assert_eq!(train.stdout_path, "/home/alice/run/slurm-%j.out");
        assert!(!train.submit_time.is_empty());
        assert!(train.array_job_id.is_empty(), "array_job_id 0 means not an array");
        assert_eq!(train.display_id(), "5001");
    }

    #[test]
    fn squeue_json_pending_and_long_running_durations() {
        let jobs = parse_squeue_json(&fixture("squeue"), "alice", NOW);
        let pending = jobs.iter().find(|j| j.job_id == "6000").unwrap();
        assert_eq!(pending.time_used, "00:00:00");
        // Started one day + 1h02m05s before NOW: squeue prints D-HH:MM:SS
        let task = jobs.iter().find(|j| j.job_id == "6004").unwrap();
        assert_eq!(task.time_used, "1-01:02:05");
    }

    #[test]
    fn squeue_json_suspended_uses_pre_sus_time() {
        let jobs = parse_squeue_json(&fixture("squeue"), "alice", NOW);
        let s = jobs.iter().find(|j| j.job_id == "7000").unwrap();
        assert_eq!(s.time_used, format_slurm_duration(5000));
    }

    #[test]
    fn squeue_json_array_display_ids_match_squeue() {
        let jobs = parse_squeue_json(&fixture("squeue"), "alice", NOW);
        let meta = jobs.iter().find(|j| j.job_id == "6000").unwrap();
        assert_eq!(meta.display_id(), "6000_[4-9%2]");
        let task = jobs.iter().find(|j| j.job_id == "6004").unwrap();
        assert_eq!(task.display_id(), "6000_3");
        assert_eq!(task.array_job_id, "6000");
    }

    #[test]
    fn squeue_json_filters_other_users() {
        // Some Slurm versions ignore -u with --json
        let jobs = parse_squeue_json(&fixture("squeue"), "alice", NOW);
        assert!(jobs.iter().all(|j| j.user == "alice"));
        assert_eq!(jobs.len(), 4);
    }

    #[test]
    fn running_job_suspended_then_resumed_counts_pre_sus_time() {
        let entry = serde_json::json!({
            "job_state": ["RUNNING"],
            "start_time": 1000,
            "suspend_time": 5000,  // resumed at 5000
            "pre_sus_time": 3000,  // ran 3000s before being suspended
        });
        assert_eq!(job_elapsed_secs(&entry, 5100), 3100);
    }

    #[test]
    fn finished_job_elapsed_is_end_minus_start() {
        let entry = serde_json::json!({
            "job_state": ["COMPLETED"],
            "start_time": {"set": true, "number": 100},
            "end_time": {"set": true, "number": 400},
        });
        assert_eq!(job_elapsed_secs(&entry, 99999), 300);
    }

    #[test]
    fn unset_envelope_is_empty_and_none() {
        let v = serde_json::json!({"set": false, "infinite": false, "number": 0});
        assert_eq!(slurm_val_to_string(&v), "");
        assert_eq!(slurm_val_to_f64(&v), None);
        let inf = serde_json::json!({"set": false, "infinite": true, "number": 0});
        assert_eq!(slurm_val_to_string(&inf), "UNLIMITED");
        assert_eq!(slurm_val_to_f64(&inf), None);
    }

    #[test]
    fn scontrol_json_form_state_reads_modern_fields() {
        let job = first_job(&fixture("scontrol")).unwrap();
        let s = extract_form_state(&job);
        assert_eq!(s["name"], "train2");
        assert_eq!(s["time"], "UNLIMITED");
        assert_eq!(s["ntasks"], "4");
        assert_eq!(s["cpus"], "8");
        // memory_per_node unset -> memory_per_cpu is a different sbatch option
        assert!(!s.contains_key("memory"));
        assert_eq!(s["extra.mem-per-cpu"], "4G");
        // pending job: GPUs come from tres_per_node
        assert_eq!(s["gpus"], "a100:2");
        assert_eq!(s["extra.account"], "proj");
        assert_eq!(s["extra.qos"], "high");
        assert_eq!(s["extra.dependency"], "afterok:4000");
        assert_eq!(s["extra.constraint"], "a100");
        assert_eq!(s["script"], "/home/alice/run/train.sh");
    }

    #[test]
    fn gpus_from_tres_handles_every_spelling() {
        assert_eq!(gpus_from_tres("gres/gpu:2"), Some("2".into()));
        assert_eq!(gpus_from_tres("gres:gpu:2"), Some("2".into()));
        assert_eq!(gpus_from_tres("gres/gpu:a100=2"), Some("a100:2".into()));
        assert_eq!(gpus_from_tres("gpu:a100:2(IDX:0-1)"), Some("a100:2".into()));
        assert_eq!(gpus_from_tres("cpu=4,gres/gpu=1"), Some("1".into()));
        assert_eq!(gpus_from_tres("gres/shard:4"), None);
        assert_eq!(gpus_from_tres(""), None);
    }

    #[test]
    fn log_path_prefers_server_expansion() {
        let job = first_job(&fixture("scontrol")).unwrap();
        assert_eq!(
            resolve_log_path(&job, false).unwrap(),
            "/home/alice/run/logs/train2-5002.out"
        );
        assert_eq!(
            resolve_log_path(&job, true).unwrap(),
            "/home/alice/run/logs/train2-5002.err"
        );
    }

    #[test]
    fn log_path_expands_patterns_and_defaults() {
        let job = serde_json::json!({
            "job_id": 42, "name": "t", "user_name": "u",
            "current_working_directory": "/w",
            "standard_output": "out-%x-%j.log",
            "standard_error": "",
        });
        assert_eq!(resolve_log_path(&job, false).unwrap(), "/w/out-t-42.log");
        // unset stderr goes to the stdout file
        assert_eq!(resolve_log_path(&job, true).unwrap(), "/w/out-t-42.log");
        // unset stdout -> sbatch default slurm-%j.out in the work dir
        let bare = serde_json::json!({"job_id": 7, "current_working_directory": "/w/"});
        assert_eq!(resolve_log_path(&bare, false).unwrap(), "/w/slurm-7.out");
        let arr = serde_json::json!({"job_id": 9, "array_job_id": 5, "array_task_id": 4,
            "current_working_directory": "/w"});
        assert_eq!(resolve_log_path(&arr, false).unwrap(), "/w/slurm-5_4.out");
    }

    #[test]
    fn sacct_json_maps_to_inspector_keys() {
        let data = fixture("sacct");
        let d = sacct_job_to_details(&data["jobs"][0]);
        assert_eq!(d["slurmterm_source"], "sacct");
        assert_eq!(job_state_of(&d), "FAILED");
        assert_eq!(slurm_val_to_string(&d["user_name"]), "alice");
        assert_eq!(slurm_val_to_string(&d["current_working_directory"]), "/home/alice/prep");
        assert_eq!(job_elapsed_secs(&d, NOW + 99999), 3725);
        assert_eq!(slurm_val_to_string(&d["exit_code"]), "1");
        assert_eq!(resolve_log_path(&d, false).unwrap(), "/home/alice/prep/prep-4000.out");
        let s = extract_form_state(&d);
        assert_eq!(s["time"], "01:00:00");
        assert_eq!(s["memory"], "8G");
        assert_eq!(s["partition"], "cpu");
    }

    #[test]
    fn sacct_without_stdout_has_no_guessed_log_path() {
        let d = sacct_job_to_details(&serde_json::json!({"job_id": 1, "state": {"current": ["COMPLETED"]}}));
        assert_eq!(resolve_log_path(&d, false), None);
    }

    #[test]
    fn parsable_job_id_formats() {
        assert_eq!(parse_parsable_job_id("123\n"), Some("123".into()));
        assert_eq!(parse_parsable_job_id("123;cluster1\n"), Some("123".into()));
        assert_eq!(parse_parsable_job_id("Submitted batch job 5"), None);
        assert_eq!(parse_parsable_job_id(""), None);
    }

    #[test]
    fn node_json_scales_cpu_load_and_maps_alloc() {
        let data = serde_json::json!({
            "nodes": [{
                "name": "n1", "state": ["MIXED"], "cpus": 64,
                "alloc_cpus": 16, "alloc_memory": 32000,
                "cpu_load": 1234, "reason": "", "gres_used": "gpu:a100:1(IDX:0)"
            }]
        });
        let f = &parse_scontrol_nodes_json(&data)[0].fields;
        assert_eq!(f["CPULoad"], "12.34");
        assert_eq!(f["CPUAlloc"], "16");
        assert_eq!(f["AllocMem"], "32000");
        assert_eq!(f["GresUsed"], "gpu:a100:1(IDX:0)");
        assert!(!f.contains_key("Reason"), "empty reason is omitted");
    }

    #[test]
    fn real_controller_rejects_unknown_signals() {
        let ctl = RealSlurmController::new(5.0);
        assert!(!ctl.signal_job("123", "SEGV; rm -rf", false));
        assert!(ctl.take_last_error().unwrap().contains("Unsupported signal"));
        assert!(!ctl.requeue_job("12 3"));
    }
}
