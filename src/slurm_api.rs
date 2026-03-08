use std::collections::HashMap;
use std::process::Command;

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
    #[allow(dead_code)]
    fn get_gpu_utilization(&self, node: Option<&str>) -> Vec<f64>;
}

// ---------------------------------------------------------------------------
// RealSlurmController
// ---------------------------------------------------------------------------

pub struct RealSlurmController;

impl RealSlurmController {
    fn run_cmd(cmd: &[&str], _timeout_secs: u64) -> (i32, String, String) {
        let result = Command::new(cmd[0])
            .args(&cmd[1..])
            .output();
        match result {
            Ok(output) => {
                let rc = output.status.code().unwrap_or(1);
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                (rc, stdout, stderr)
            }
            Err(_) => (127, String::new(), format!("{}: command not found", cmd[0])),
        }
    }
}

impl SlurmController for RealSlurmController {
    fn current_user(&self) -> String {
        std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "unknown".to_string())
    }

    fn get_cluster_name(&self) -> String {
        let (rc, stdout, _) = Self::run_cmd(&["scontrol", "show", "config"], 30);
        if rc != 0 {
            return "unknown".to_string();
        }
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
        let (rc, stdout, _) =
            Self::run_cmd(&["squeue", "-u", &user, "--json"], 30);
        if rc != 0 {
            return Vec::new();
        }
        let data: serde_json::Value = match serde_json::from_str(&stdout) {
            Ok(v) => v,
            Err(_) => return Vec::new(),
        };
        let jobs = match data.get("jobs").and_then(|j| j.as_array()) {
            Some(arr) => arr,
            None => return Vec::new(),
        };
        jobs.iter().map(parse_job_entry).collect()
    }

    fn get_partitions(&self) -> Vec<String> {
        let (rc, stdout, _) = Self::run_cmd(&["sinfo", "-h", "-o", "%P"], 30);
        if rc != 0 {
            return Vec::new();
        }
        stdout
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| l.trim().trim_end_matches('*').to_string())
            .collect()
    }

    fn get_job_details(&self, job_id: &str) -> Option<serde_json::Value> {
        let job_id = validate_job_id(job_id).ok()?;
        let (rc, stdout, _) =
            Self::run_cmd(&["scontrol", "show", "job", &job_id, "--json"], 30);
        if rc != 0 {
            return None;
        }
        let data: serde_json::Value = serde_json::from_str(&stdout).ok()?;
        data.get("jobs")
            .and_then(|j| j.as_array())
            .and_then(|arr| arr.first().cloned())
    }

    fn cancel_job(&self, job_id: &str) -> bool {
        let Ok(job_id) = validate_job_id(job_id) else {
            return false;
        };
        let (rc, _, _) = Self::run_cmd(&["scancel", &job_id], 30);
        rc == 0
    }

    fn hold_job(&self, job_id: &str) -> bool {
        let Ok(job_id) = validate_job_id(job_id) else {
            return false;
        };
        let (rc, _, _) = Self::run_cmd(&["scontrol", "hold", &job_id], 30);
        rc == 0
    }

    fn release_job(&self, job_id: &str) -> bool {
        let Ok(job_id) = validate_job_id(job_id) else {
            return false;
        };
        let (rc, _, _) = Self::run_cmd(&["scontrol", "release", &job_id], 30);
        rc == 0
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
        let (rc, stdout, stderr) = Self::run_cmd(&refs, 30);
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
        let (rc, stdout, _) = Self::run_cmd(&["sinfo", "-h", "-o", fmt], 30);
        if rc != 0 {
            return Vec::new();
        }
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
        let (rc, stdout, _) = Self::run_cmd(&["scontrol", "show", "nodes"], 30);
        if rc != 0 {
            return Vec::new();
        }
        let re = Regex::new(r"(\w+)=(\S*)").unwrap();
        let mut nodes = Vec::new();
        let mut current = HashMap::new();
        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                if !current.is_empty() {
                    nodes.push(NodeInfoRow {
                        fields: std::mem::take(&mut current),
                    });
                }
                continue;
            }
            for cap in re.captures_iter(line) {
                current.insert(
                    cap[1].to_string(),
                    cap[2].trim().to_string(),
                );
            }
        }
        if !current.is_empty() {
            nodes.push(NodeInfoRow { fields: current });
        }
        nodes
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
        let (rc, stdout, _) = Self::run_cmd(&full_args, 30);
        if rc != 0 {
            return Vec::new();
        }
        stdout
            .lines()
            .filter_map(|line| {
                let parts: Vec<&str> = line.split('|').collect();
                if parts.len() >= 8 {
                    let job_id = parts[0].trim().to_string();
                    if job_id.contains('.') {
                        return None;
                    }
                    Some(SacctRow {
                        job_id,
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
            .collect()
    }

    fn get_sstat(&self, job_id: &str) -> Option<SstatResult> {
        let Ok(job_id) = validate_job_id(job_id) else {
            return None;
        };
        for suffix in &["", ".batch"] {
            let jid = format!("{job_id}{suffix}");
            let (rc, stdout, _) = Self::run_cmd(
                &["sstat", "-n", "-P", "--format=AveCPU,MaxRSS,MaxVMSize", "-j", &jid],
                30,
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
        let (rc, stdout, _) = Self::run_cmd(&["df", "-h"], 10);
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
        let base = &[
            "nvidia-smi",
            "--query-gpu=utilization.gpu",
            "--format=csv,noheader,nounits",
        ];
        let (rc, stdout, _) = if let Some(n) = node {
            let mut cmd = vec!["ssh", n];
            cmd.extend_from_slice(base);
            Self::run_cmd(&cmd, 10)
        } else {
            Self::run_cmd(base, 10)
        };
        if rc != 0 {
            return Vec::new();
        }
        stdout
            .lines()
            .filter_map(|l| l.trim().parse::<f64>().ok())
            .collect()
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

    let state_raw = entry.get("job_state");
    let state = match state_raw {
        Some(serde_json::Value::Array(arr)) => arr
            .first()
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN")
            .to_string(),
        Some(serde_json::Value::String(s)) => s.clone(),
        _ => "UNKNOWN".to_string(),
    };

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
