use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use regex::Regex;
use serde::{Deserialize, Serialize};

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
    pub extra: HashMap<String, serde_json::Value>,
}

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

fn validate_job_id(job_id: &str) -> Result<String, String> {
    let job_id = job_id.trim();
    let re = Regex::new(r"^\d+(_\d+)?$").unwrap();
    if !re.is_match(job_id) {
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
    let re = Regex::new(r"^[a-zA-Z][a-zA-Z0-9_-]*$").unwrap();
    if !re.is_match(key) {
        return Err(format!("Unsafe parameter key: {key:?}"));
    }
    Ok(())
}

fn validate_safe_filter(value: &str) -> Result<(), String> {
    let re = Regex::new(r"^[a-zA-Z0-9_.@:+/-]+$").unwrap();
    if !re.is_match(value) {
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
    ) -> Result<String, String>;
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
        let jobs = match data.get("jobs").and_then(|j| j.as_array()) {
            Some(arr) => arr,
            None => return Vec::new(),
        };
        jobs.iter().map(parse_job_entry).collect()
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
        let stdout = self.run_checked(&["scontrol", "show", "job", &job_id, "--json"])?;
        let data: serde_json::Value = serde_json::from_str(&stdout).ok()?;
        data.get("jobs")
            .and_then(|j| j.as_array())
            .and_then(|arr| arr.first().cloned())
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

    fn submit_job(
        &self,
        script_path: &str,
        params: &HashMap<String, String>,
    ) -> Result<String, String> {
        let mut args: Vec<String> = vec!["sbatch".to_string()];
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
        stdout
            .split_whitespace()
            .last()
            .map(|s| s.to_string())
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
        let (rc, stdout, _) = self.run_cmd(&["df", "-h"], self.timeout_secs.min(10));
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

// ---------------------------------------------------------------------------
// Parse squeue JSON job entry
// ---------------------------------------------------------------------------

fn parse_job_entry(entry: &serde_json::Value) -> JobInfo {
    let time_raw = entry.get("time");
    let time_used = match time_raw {
        Some(serde_json::Value::Object(map)) => {
            let elapsed = map
                .get("elapsed")
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let h = elapsed / 3600;
            let m = (elapsed % 3600) / 60;
            let s = elapsed % 60;
            format!("{h:02}:{m:02}:{s:02}")
        }
        Some(v) => v.to_string().trim_matches('"').to_string(),
        None => String::new(),
    };

    let state = job_state_of(entry);

    let node_count = match entry.get("node_count") {
        Some(serde_json::Value::Object(map)) => map
            .get("number")
            .map(|v| v.to_string().trim_matches('"').to_string())
            .unwrap_or_default(),
        Some(v) => v.to_string().trim_matches('"').to_string(),
        None => String::new(),
    };

    let submit_time = match entry.get("submit_time") {
        Some(serde_json::Value::Object(map)) => map
            .get("number")
            .map(|v| v.to_string().trim_matches('"').to_string())
            .unwrap_or_default(),
        Some(v) => v.to_string().trim_matches('"').to_string(),
        None => String::new(),
    };

    let get_str = |key: &str| -> String {
        entry
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };
    let get_str_or = |key: &str| -> String {
        match entry.get(key) {
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(v) => v.to_string().trim_matches('"').to_string(),
            None => String::new(),
        }
    };

    JobInfo {
        job_id: get_str_or("job_id"),
        name: get_str("name"),
        partition: get_str("partition"),
        state,
        time_used,
        nodes: if node_count.is_empty() {
            get_str_or("nodes")
        } else {
            node_count
        },
        reason: get_str_or("state_reason"),
        user: get_str("user_name"),
        work_dir: get_str("working_directory"),
        stdout_path: get_str("standard_output"),
        stderr_path: get_str("standard_error"),
        submit_time,
        node_list: get_str_or("nodes"),
        extra: HashMap::new(),
    }
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
            for (field, json_key) in [
                ("NodeName", "name"),
                ("CPUTot", "cpus"),
                ("RealMemory", "real_memory"),
                ("FreeMem", "free_mem"),
                ("Gres", "gres"),
                ("Partitions", "partitions"),
                ("CPULoad", "cpu_load"),
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

/// Extract form state from scontrol job details JSON for resubmission.
pub fn extract_form_state(details: &serde_json::Value) -> HashMap<String, String> {
    let mut state = HashMap::new();
    state.insert("mode".into(), "sbatch".into());

    let get_str = |key: &str| -> String {
        details
            .get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };

    state.insert("name".into(), get_str("name"));
    state.insert("partition".into(), get_str("partition"));

    // Time limit (minutes -> HH:MM:SS)
    let tl = match details.get("time_limit") {
        Some(serde_json::Value::Object(map)) => map
            .get("number")
            .and_then(|v| v.as_i64())
            .unwrap_or(0),
        Some(v) => v.as_i64().unwrap_or(0),
        None => 0,
    };
    if tl > 0 {
        let total_sec = tl * 60;
        let h = total_sec / 3600;
        let m = (total_sec % 3600) / 60;
        let s = total_sec % 60;
        state.insert("time".into(), format!("{h:02}:{m:02}:{s:02}"));
    }

    let nodes = match details.get("node_count") {
        Some(serde_json::Value::Object(map)) => map
            .get("number")
            .map(|v| v.to_string().trim_matches('"').to_string())
            .unwrap_or("1".into()),
        Some(v) => v.to_string().trim_matches('"').to_string(),
        None => "1".into(),
    };
    state.insert("nodes".into(), nodes);

    let ntasks = match details.get("tasks_per_node") {
        Some(serde_json::Value::Object(map)) => map
            .get("number")
            .map(|v| v.to_string().trim_matches('"').to_string())
            .unwrap_or_default(),
        Some(v) => v.to_string().trim_matches('"').to_string(),
        None => String::new(),
    };
    state.insert("ntasks".into(), ntasks);

    let cpus = match details.get("cpus_per_task") {
        Some(serde_json::Value::Object(map)) => map
            .get("number")
            .map(|v| v.to_string().trim_matches('"').to_string())
            .unwrap_or_default(),
        Some(v) => v.to_string().trim_matches('"').to_string(),
        None => String::new(),
    };
    state.insert("cpus".into(), cpus);

    let mem = match details.get("minimum_memory_per_node") {
        Some(serde_json::Value::Object(map)) => map
            .get("number")
            .map(|v| v.to_string().trim_matches('"').to_string())
            .unwrap_or_default(),
        Some(v) => v.to_string().trim_matches('"').to_string(),
        None => String::new(),
    };
    if !mem.is_empty() {
        if let Ok(mb) = mem.parse::<f64>() {
            let gb = mb / 1024.0;
            if gb >= 1.0 {
                state.insert("memory".into(), format!("{gb:.0}G"));
            } else {
                state.insert("memory".into(), format!("{mem}M"));
            }
        }
    }

    let gres = get_str("gres_detail");
    if !gres.is_empty() && gres != "(null)" && gres != "[]" {
        state.insert("gpus".into(), gres);
    }

    state.insert("script".into(), get_str("command"));
    state.insert("output".into(), get_str("standard_output"));
    state.insert("error".into(), get_str("standard_error"));

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
        let job = parse_job_entry(&entry);
        assert_eq!(job.job_id, "4242");
        assert_eq!(job.state, "RUNNING");
        assert_eq!(job.time_used, "01:02:05");
        assert_eq!(job.nodes, "2");
        assert_eq!(job.node_list, "gpu[001-002]");
    }
}
