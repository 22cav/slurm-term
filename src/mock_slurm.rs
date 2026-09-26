use std::cell::RefCell;
use std::collections::HashMap;

use crate::slurm_api::{
    merge_sacct_steps, now_epoch, parse_job_entry, sacct_job_to_details, JobInfo, NodeInfoRow,
    SacctRow, SinfoRow, SlurmController, SstatResult, StorageInfo, SubmitOutcome, JOB_SIGNALS,
};
use crate::validators::format_hms;

const PARTITIONS: &[&str] = &["debug", "batch", "gpu", "bigmem"];
const JOB_NAMES: &[&str] = &[
    "train_resnet50", "preprocess_data", "eval_model", "hyperopt_search",
    "feature_extract", "run_simulation", "postprocess", "benchmark_v2",
    "data_augment", "inference_batch",
];
const REASONS: &[&str] = &["None", "Resources", "Priority", "QOSMaxJobsPerUserLimit", "Dependency"];

struct MockJob {
    id: String,
    name: String,
    partition: String,
    state: String,
    user: String,
    elapsed: i64,
    nodes: String,
    reason: String,
    node_list: String,
    log_path: String,
    time_limit: i64,
    metrics: MockMetrics,
    born: std::time::Instant,
    /// Array master ID, for array tasks and the pending array record.
    array_job_id: Option<String>,
    /// Task index of a started array task.
    array_task_id: Option<u32>,
    /// Unstarted tasks of the pending array record ("4-9%2").
    array_pending: Option<String>,
}

/// One finished job as the mock's accounting database stores it.
struct MockHistoryJob {
    rows: Vec<SacctRow>,
    /// The job in `sacct --json` layout.
    json: serde_json::Value,
}

/// Wrap a scalar in Slurm's modern `{"set", "infinite", "number"}` envelope.
fn num(n: i64) -> serde_json::Value {
    serde_json::json!({"set": true, "infinite": false, "number": n})
}

#[derive(Clone)]
struct MockMetrics {
    cpu: Vec<f64>,
    mem: Vec<f64>,
    gpu: Vec<f64>,
}

/// Simple deterministic PRNG (xorshift64).
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(if seed == 0 { 1 } else { seed })
    }
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        if hi <= lo { return lo; }
        lo + (self.next_u64() % (hi - lo) as u64) as i64
    }
    fn urange(&mut self, lo: usize, hi: usize) -> usize {
        if hi <= lo { return lo; }
        lo + (self.next_u64() as usize) % (hi - lo)
    }
    fn frange(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (self.next_u64() as f64 / u64::MAX as f64) * (hi - lo)
    }
    fn chance(&mut self, pct: f64) -> bool {
        self.frange(0.0, 1.0) < pct
    }
    fn choice<'a>(&mut self, items: &'a [&str]) -> &'a str {
        if items.is_empty() {
            return "";
        }
        items[self.urange(0, items.len())]
    }
}

pub struct MockSlurmController {
    inner: RefCell<MockInner>,
}

struct MockInner {
    jobs: Vec<MockJob>,
    next_id: u64,
    rng: Rng,
    cancelled: std::collections::HashSet<String>,
    held: std::collections::HashSet<String>,
    tmpdir: String,
    history: Vec<MockHistoryJob>,
}

impl MockSlurmController {
    pub fn new(num_jobs: usize, seed: Option<u64>) -> Self {
        let tmpdir = std::env::temp_dir()
            .join("slurmterm_demo")
            .to_string_lossy()
            .to_string();
        let _ = std::fs::create_dir_all(&tmpdir);

        let mut inner = MockInner {
            jobs: Vec::new(),
            next_id: 100001,
            rng: Rng::new(seed.unwrap_or(42)),
            cancelled: std::collections::HashSet::new(),
            held: std::collections::HashSet::new(),
            tmpdir,
            history: Vec::new(),
        };
        for _ in 0..num_jobs {
            inner.spawn_job(None);
        }
        if num_jobs > 0 {
            inner.spawn_array(4, 10);
        }
        inner.gen_history(15);
        Self { inner: RefCell::new(inner) }
    }
}

impl MockInner {
    fn spawn_job(&mut self, state: Option<&str>) -> String {
        let job_id = self.next_id.to_string();
        self.next_id += 1;

        let state = state.unwrap_or_else(|| {
            let states = &["RUNNING", "RUNNING", "RUNNING", "PENDING", "PENDING", "COMPLETING"];
            self.rng.choice(states)
        }).to_string();

        let name = self.rng.choice(JOB_NAMES).to_string();
        let partition = self.rng.choice(PARTITIONS).to_string();
        let user_choices = &["matte", "alice", "bob"];
        let user = self.rng.choice(user_choices).to_string();
        let elapsed = if matches!(state.as_str(), "RUNNING" | "COMPLETING") {
            self.rng.range(0, 36000)
        } else {
            0
        };
        let nodes = self.rng.range(1, 9).to_string();
        let reason = if state == "RUNNING" {
            "None".to_string()
        } else {
            self.rng.choice(REASONS).to_string()
        };
        let a = self.rng.range(1, 50);
        let b = self.rng.range(51, 100);
        let node_list = format!("node[{a:03}-{b:03}]");
        let time_limit = [3600i64, 7200, 14400, 86400][self.rng.urange(0, 4)];

        let log_path = format!("{}/slurm-{job_id}.out", self.tmpdir);
        let err_path = log_path.replace(".out", ".err");
        let _ = std::fs::write(&log_path, format!("=== SLURM Job {job_id} ===\nStarted.\n\n"));
        let _ = std::fs::write(&err_path, format!("=== SLURM Job {job_id} stderr ===\n"));

        let metrics = self.gen_metrics();

        self.jobs.push(MockJob {
            id: job_id.clone(),
            name,
            partition,
            state,
            user,
            elapsed,
            nodes,
            reason,
            node_list,
            log_path,
            time_limit,
            metrics,
            born: std::time::Instant::now(),
            array_job_id: None,
            array_task_id: None,
            array_pending: None,
        });
        job_id
    }

    /// A job array like `sbatch --array=0-<total-1>%<running>`: `running`
    /// started tasks plus the pending record holding the rest, laid out the
    /// way squeue reports them (the pending record keeps the master ID).
    fn spawn_array(&mut self, running: u32, total: u32) {
        let master = self.spawn_job(Some("PENDING"));
        let name = "sweep_lr".to_string();
        let mi = self.find_job(&master).unwrap();
        self.jobs[mi].name = name.clone();
        self.jobs[mi].reason = "JobArrayTaskLimit".into();
        self.jobs[mi].nodes = "1".into();
        self.jobs[mi].array_job_id = Some(master.clone());
        self.jobs[mi].array_pending = Some(format!("{running}-{}%{running}", total - 1));
        for t in 0..running {
            let id = self.spawn_job(Some("RUNNING"));
            let i = self.find_job(&id).unwrap();
            self.jobs[i].name = name.clone();
            self.jobs[i].partition = self.jobs[mi].partition.clone();
            self.jobs[i].nodes = "1".into();
            self.jobs[i].array_job_id = Some(master.clone());
            self.jobs[i].array_task_id = Some(t);
        }
    }

    fn gen_history(&mut self, count: u64) {
        let now = now_epoch();
        let user = std::env::var("USER").unwrap_or_else(|_| "matte".into());
        for i in 0..count {
            let jid = format!("{}", 99900 + i);
            let elapsed_s = self.rng.range(120, 86400);
            let elapsed = format_hms(elapsed_s);
            let state_choices = &[
                "COMPLETED", "COMPLETED", "COMPLETED", "COMPLETED", "COMPLETED", "COMPLETED",
                "FAILED", "FAILED", "TIMEOUT", "CANCELLED",
            ];
            let state = self.rng.choice(state_choices).to_string();
            let rc = if state == "COMPLETED" { 0 } else { self.rng.range(1, 128) };
            let exit_code = format!("{rc}:0");
            let name = self.rng.choice(JOB_NAMES).to_string();
            let partition = self.rng.choice(PARTITIONS).to_string();
            // Like real sacct: the job row has no MaxRSS/TotalCPU — only the
            // .batch step row carries them (in kilobytes). This exercises the
            // same step-merge logic used for real cluster output.
            let max_rss_kb = format!("{}K", self.rng.range(500, 64000) * 1024);
            let rows = vec![
                SacctRow {
                    job_id: jid.clone(),
                    name: name.clone(),
                    partition: partition.clone(),
                    state: state.clone(),
                    elapsed: elapsed.clone(),
                    total_cpu: String::new(),
                    max_rss: String::new(),
                    exit_code: exit_code.clone(),
                },
                SacctRow {
                    job_id: format!("{jid}.batch"),
                    name: "batch".to_string(),
                    partition: String::new(),
                    state: state.clone(),
                    elapsed,
                    total_cpu: "01:23:45".into(),
                    max_rss: max_rss_kb,
                    exit_code,
                },
            ];
            let end = now - (count as i64 - i as i64) * 3600;
            let start = end - elapsed_s;
            let log_path = format!("{}/slurm-{jid}.out", self.tmpdir);
            let _ = std::fs::write(
                &log_path,
                format!("=== SLURM Job {jid} ===\nStarted.\nFinished: {state}\n"),
            );
            let json = serde_json::json!({
                "job_id": jid.parse::<i64>().unwrap_or(0),
                "name": name,
                "partition": partition,
                "user": user,
                "account": "demo",
                "qos": "normal",
                "nodes": "node007",
                "state": {"current": [state], "reason": "None"},
                "time": {
                    "elapsed": elapsed_s,
                    "limit": num(1440),
                    "start": start,
                    "end": end,
                    "submission": start - 60,
                },
                "working_directory": format!("/home/{user}/projects"),
                "stdout": format!("{}/slurm-%j.out", self.tmpdir),
                "stderr": "",
                "stdout_expanded": log_path,
                "required": {"CPUs": 8, "memory_per_node": num(16384), "memory_per_cpu": {"set": false, "infinite": false, "number": 0}},
                "exit_code": {"status": ["EXITED"], "return_code": num(rc)},
                "array": {"job_id": 0, "task_id": {"set": false, "infinite": false, "number": 0}},
            });
            self.history.push(MockHistoryJob { rows, json });
        }
    }

    /// The job in `squeue --json` / `scontrol show job --json` layout.
    fn job_json(&self, j: &MockJob, now: i64) -> serde_json::Value {
        let started = !matches!(j.state.as_str(), "PENDING");
        let start = if started { now - j.elapsed } else { 0 };
        let (array_job_id, array_task_id, array_task_string) = match &j.array_job_id {
            Some(a) => (
                a.parse::<i64>().unwrap_or(0),
                match j.array_task_id {
                    Some(t) => num(t as i64),
                    None => serde_json::json!({"set": false, "infinite": false, "number": 0}),
                },
                j.array_pending.clone().unwrap_or_default(),
            ),
            None => (0, serde_json::json!({"set": false, "infinite": false, "number": 0}), String::new()),
        };
        serde_json::json!({
            "job_id": j.id.parse::<i64>().unwrap_or(0),
            "name": j.name,
            "job_state": [j.state],
            "partition": j.partition,
            "user_name": j.user,
            "state_reason": j.reason,
            "current_working_directory": format!("/home/{}/projects/{}", j.user, j.name),
            "nodes": if started { j.node_list.clone() } else { String::new() },
            "node_count": num(j.nodes.parse().unwrap_or(1)),
            "standard_output": format!("{}/slurm-%j.out", self.tmpdir),
            "standard_error": format!("{}/slurm-%j.err", self.tmpdir),
            "stdout_expanded": j.log_path,
            "stderr_expanded": j.log_path.replace(".out", ".err"),
            "submit_time": num(now - j.elapsed - 120),
            "start_time": num(start),
            // Finished jobs linger in squeue for MinJobAge with an end time
            "end_time": num(if matches!(j.state.as_str(), "COMPLETED" | "FAILED" | "TIMEOUT" | "CANCELLED") {
                now
            } else {
                0
            }),
            "suspend_time": num(0),
            "pre_sus_time": num(0),
            "time_limit": num(j.time_limit / 60),
            "array_job_id": num(array_job_id),
            "array_task_id": array_task_id,
            "array_task_string": array_task_string,
        })
    }

    fn gen_metrics(&mut self) -> MockMetrics {
        let mut cpu = Vec::new();
        let mut mem = Vec::new();
        let mut gpu = Vec::new();
        let mut cv = self.rng.frange(30.0, 80.0);
        let mut mv = self.rng.frange(30.0, 70.0);
        let mut gv = self.rng.frange(15.0, 75.0);
        for _ in 0..30 {
            cpu.push(cv);
            mem.push(mv);
            gpu.push(gv);
            cv = (cv + self.rng.frange(-5.0, 5.0)).clamp(0.0, 100.0);
            mv = (mv + self.rng.frange(-5.0, 5.0)).clamp(0.0, 100.0);
            gv = (gv + self.rng.frange(-5.0, 5.0)).clamp(0.0, 100.0);
        }
        MockMetrics { cpu, mem, gpu }
    }

    fn tick(&mut self) {
        let n = self.jobs.len();
        for i in 0..n {
            let id = self.jobs[i].id.clone();
            if self.cancelled.contains(&id)
                || self.held.contains(&id)
                || self.jobs[i].array_pending.is_some()
            {
                continue;
            }
            let age = self.jobs[i].born.elapsed().as_secs_f64();

            match self.jobs[i].state.as_str() {
                "RUNNING" => {
                    self.jobs[i].elapsed += 3;
                    // Update metrics
                    for metric in ["cpu", "mem", "gpu"] {
                        let data = match metric {
                            "cpu" => &mut self.jobs[i].metrics.cpu,
                            "mem" => &mut self.jobs[i].metrics.mem,
                            _ => &mut self.jobs[i].metrics.gpu,
                        };
                        let last = *data.last().unwrap_or(&50.0);
                        let new = (last + self.rng.frange(-10.0, 10.0)).clamp(0.0, 100.0);
                        data.push(new);
                        if data.len() > 60 { data.remove(0); }
                    }
                    // Write log
                    let line = format!("Processing step {} ...\n", self.jobs[i].elapsed / 3);
                    let _ = std::fs::OpenOptions::new()
                        .append(true)
                        .open(&self.jobs[i].log_path)
                        .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));

                    if age > 20.0 && self.rng.chance(0.08) {
                        let new_state = ["COMPLETED", "COMPLETED", "FAILED", "TIMEOUT"]
                            [self.rng.urange(0, 4)]
                        .to_string();
                        self.jobs[i].state = new_state;
                    } else if age > 15.0 && self.rng.chance(0.05) {
                        self.jobs[i].state = "COMPLETING".to_string();
                    }
                }
                "COMPLETING" if self.rng.chance(0.4) => {
                    self.jobs[i].state = "COMPLETED".to_string();
                }
                "PENDING" if age > 10.0 && self.rng.chance(0.15) => {
                    self.jobs[i].state = "RUNNING".to_string();
                }
                _ => {}
            }
        }

        // Remove finished jobs and maybe spawn new ones
        self.jobs.retain(|j| {
            !matches!(j.state.as_str(), "COMPLETED" | "FAILED" | "TIMEOUT" | "CANCELLED")
                || j.born.elapsed().as_secs() < 30
        });

        if self.jobs.len() < 5 && self.rng.chance(0.3) {
            self.spawn_job(None);
        }
    }

    fn find_job(&self, job_id: &str) -> Option<usize> {
        self.jobs.iter().position(|j| j.id == job_id)
    }
}

impl SlurmController for MockSlurmController {
    fn current_user(&self) -> String {
        std::env::var("USER").unwrap_or_else(|_| "matte".into())
    }

    fn get_cluster_name(&self) -> String {
        "demo-cluster".to_string()
    }

    fn supports_interactive(&self) -> bool {
        false
    }

    fn get_queue(&self, _user: Option<&str>) -> Vec<JobInfo> {
        let mut inner = self.inner.borrow_mut();
        inner.tick();
        // Round-trip through the real squeue JSON parser so demo mode
        // exercises the same code path as a cluster. The demo shows every
        // user's jobs (no -u filtering) to make the queue look populated.
        let now = now_epoch();
        inner.jobs
            .iter()
            .map(|j| parse_job_entry(&inner.job_json(j, now), now))
            .collect()
    }

    fn get_partitions(&self) -> Vec<String> {
        PARTITIONS.iter().map(|s| s.to_string()).collect()
    }

    fn get_job_details(&self, job_id: &str) -> Option<serde_json::Value> {
        let inner = self.inner.borrow();
        let Some(j) = inner.jobs.iter().find(|j| j.id == job_id) else {
            // Like the real controller: jobs slurmctld no longer knows are
            // looked up in accounting (sacct --json) instead.
            return inner
                .history
                .iter()
                .find(|h| h.rows.first().is_some_and(|r| r.job_id == job_id))
                .map(|h| sacct_job_to_details(&h.json));
        };
        let profile = match j.partition.as_str() {
            "debug" => (4, 16_384, ""),
            "gpu" => (8, 32_768, "gres/gpu:a100=1"),
            "bigmem" => (32, 262_144, ""),
            _ => (16, 65_536, ""),
        };

        // Mirror the shapes modern Slurm --json actually emits (job_state as
        // an array, scalars wrapped in {number,set,infinite}, epochs for
        // times) so demo mode exercises the same handling as a real cluster.
        let mut details = inner.job_json(j, now_epoch());
        let obj = details.as_object_mut()?;
        obj.insert("cpus_per_task".into(), num(profile.0));
        obj.insert("memory_per_node".into(), num(profile.1));
        obj.insert("tres_per_node".into(), serde_json::json!(profile.2));
        if j.state == "RUNNING" && !profile.2.is_empty() {
            obj.insert("gres_detail".into(), serde_json::json!(["gpu:a100:1(IDX:0)"]));
        }
        obj.insert("account".into(), serde_json::json!("demo"));
        obj.insert("qos".into(), serde_json::json!("normal"));
        obj.insert(
            "slurmterm_metrics".into(),
            serde_json::json!({
                "cpu": j.metrics.cpu,
                "mem": j.metrics.mem,
                "gpu": j.metrics.gpu,
            }),
        );
        Some(details)
    }

    fn cancel_job(&self, job_id: &str) -> bool {
        let mut inner = self.inner.borrow_mut();
        // Like scancel: an array master ID cancels every task of the array.
        let targets: Vec<usize> = inner
            .jobs
            .iter()
            .enumerate()
            .filter(|(_, j)| j.id == job_id || j.array_job_id.as_deref() == Some(job_id))
            .map(|(i, _)| i)
            .collect();
        for &i in &targets {
            let id = inner.jobs[i].id.clone();
            inner.cancelled.insert(id);
            inner.jobs[i].state = "CANCELLED".to_string();
        }
        !targets.is_empty()
    }

    fn hold_job(&self, job_id: &str) -> bool {
        let mut inner = self.inner.borrow_mut();
        if let Some(idx) = inner.find_job(job_id) {
            if inner.jobs[idx].state == "PENDING" {
                inner.held.insert(job_id.to_string());
                return true;
            }
        }
        false
    }

    fn release_job(&self, job_id: &str) -> bool {
        let mut inner = self.inner.borrow_mut();
        inner.held.remove(job_id)
    }

    fn requeue_job(&self, job_id: &str) -> bool {
        let mut inner = self.inner.borrow_mut();
        let Some(i) = inner.find_job(job_id) else {
            return false;
        };
        if inner.jobs[i].state == "PENDING" {
            return false; // scontrol: "Job is pending execution"
        }
        inner.cancelled.remove(job_id);
        let j = &mut inner.jobs[i];
        j.state = "PENDING".into();
        j.reason = "BeginTime".into();
        j.elapsed = 0;
        j.born = std::time::Instant::now();
        true
    }

    fn signal_job(&self, job_id: &str, signal: &str, batch_only: bool) -> bool {
        if !JOB_SIGNALS.contains(&signal) {
            return false;
        }
        let inner = self.inner.borrow();
        let Some(j) = inner.jobs.iter().find(|j| j.id == job_id) else {
            return false;
        };
        if j.state != "RUNNING" {
            return false;
        }
        let scope = if batch_only { "batch step" } else { "all steps" };
        let _ = std::fs::OpenOptions::new()
            .append(true)
            .open(&j.log_path)
            .and_then(|mut f| {
                std::io::Write::write_all(
                    &mut f,
                    format!("slurmstepd: received SIG{signal} ({scope})\n").as_bytes(),
                )
            });
        true
    }

    fn submit_job(
        &self,
        _script_path: &str,
        params: &HashMap<String, String>,
    ) -> Result<SubmitOutcome, String> {
        let mut inner = self.inner.borrow_mut();
        if params.contains_key("test-only") {
            let part = params.get("partition").cloned().unwrap_or_else(|| "batch".into());
            return Ok(SubmitOutcome::TestOnly(format!(
                "Job {} to start at now using 1 processors on nodes node001 in partition {part}",
                inner.next_id
            )));
        }
        let id = inner.spawn_job(Some("PENDING"));
        Ok(SubmitOutcome::Submitted(id))
    }

    fn get_sinfo(&self) -> Vec<SinfoRow> {
        vec![
            SinfoRow {
                partition: "debug".into(), avail: "up".into(), timelimit: "00:30:00".into(),
                nodes: "4".into(), state: "idle".into(), nodelist: "node[001-004]".into(),
                cpus: "16".into(), memory: "64000".into(), gres: "(null)".into(),
            },
            SinfoRow {
                partition: "batch".into(), avail: "up".into(), timelimit: "7-00:00:00".into(),
                nodes: "20".into(), state: "mixed".into(), nodelist: "node[005-024]".into(),
                cpus: "64".into(), memory: "256000".into(), gres: "(null)".into(),
            },
            SinfoRow {
                partition: "gpu".into(), avail: "up".into(), timelimit: "3-00:00:00".into(),
                nodes: "8".into(), state: "mixed".into(), nodelist: "gpu[001-008]".into(),
                cpus: "32".into(), memory: "128000".into(), gres: "gpu:a100:4".into(),
            },
            SinfoRow {
                partition: "gpu".into(), avail: "up".into(), timelimit: "3-00:00:00".into(),
                nodes: "4".into(), state: "idle".into(), nodelist: "gpu[009-012]".into(),
                cpus: "32".into(), memory: "128000".into(), gres: "gpu:a100:4".into(),
            },
            SinfoRow {
                partition: "bigmem".into(), avail: "up".into(), timelimit: "2-00:00:00".into(),
                nodes: "2".into(), state: "idle".into(), nodelist: "bigmem[001-002]".into(),
                cpus: "128".into(), memory: "1024000".into(), gres: "(null)".into(),
            },
        ]
    }

    fn get_node_info(&self) -> Vec<NodeInfoRow> {
        let mut nodes = Vec::new();
        for i in 1..=4 {
            nodes.push(NodeInfoRow {
                fields: HashMap::from([
                    ("NodeName".into(), format!("node{i:03}")),
                    ("State".into(), "IDLE".into()),
                    ("CPUTot".into(), "16".into()),
                    ("RealMemory".into(), "64000".into()),
                    ("Gres".into(), "(null)".into()),
                    ("Partitions".into(), "debug".into()),
                    ("CPULoad".into(), "0.00".into()),
                    ("FreeMem".into(), "62000".into()),
                ]),
            });
        }
        for i in 5..=32 {
            nodes.push(NodeInfoRow {
                fields: HashMap::from([
                    ("NodeName".into(), format!("node{i:03}")),
                    ("State".into(), "MIXED".into()),
                    ("CPUAlloc".into(), "32".into()),
                    ("CPUTot".into(), "64".into()),
                    ("RealMemory".into(), "256000".into()),
                    ("Gres".into(), "(null)".into()),
                    ("Partitions".into(), "batch".into()),
                    ("CPULoad".into(), "25.00".into()),
                    ("FreeMem".into(), "128000".into()),
                ]),
            });
        }
        for i in 1..=12 {
            nodes.push(NodeInfoRow {
                fields: HashMap::from([
                    ("NodeName".into(), format!("gpu{i:03}")),
                    ("State".into(), if i <= 8 { "MIXED" } else { "IDLE" }.into()),
                    ("CPUAlloc".into(), if i <= 8 { "16" } else { "0" }.into()),
                    ("CPUTot".into(), "32".into()),
                    ("RealMemory".into(), "128000".into()),
                    ("Gres".into(), "gpu:a100:4".into()),
                    ("Partitions".into(), "gpu".into()),
                    ("CPULoad".into(), "10.00".into()),
                    ("FreeMem".into(), "90000".into()),
                ]),
            });
        }
        for i in 1..=2 {
            nodes.push(NodeInfoRow {
                fields: HashMap::from([
                    ("NodeName".into(), format!("bigmem{i:03}")),
                    ("State".into(), "IDLE".into()),
                    ("CPUTot".into(), "128".into()),
                    ("RealMemory".into(), "1024000".into()),
                    ("Gres".into(), "(null)".into()),
                    ("Partitions".into(), "bigmem".into()),
                    ("CPULoad".into(), "0.00".into()),
                    ("FreeMem".into(), "1020000".into()),
                ]),
            });
        }
        nodes
    }

    fn get_sacct(&self, _user: Option<&str>, _start_time: Option<&str>) -> Vec<SacctRow> {
        let inner = self.inner.borrow();
        let rows = inner.history.iter().flat_map(|h| h.rows.iter().cloned()).collect();
        merge_sacct_steps(rows)
    }

    fn get_sstat(&self, job_id: &str) -> Option<SstatResult> {
        let inner = self.inner.borrow();
        let j = inner.jobs.iter().find(|j| j.id == job_id)?;
        if j.state != "RUNNING" {
            return None;
        }
        let cpu_pct = j.metrics.cpu.last().copied().unwrap_or(50.0);
        let mem_mb = (j.metrics.mem.last().copied().unwrap_or(50.0) / 100.0 * 32000.0) as i64;
        // Real sstat formats: AveCPU is a duration, MaxRSS is in kilobytes.
        let cpu_secs = (cpu_pct / 100.0 * j.elapsed as f64) as i64;
        Some(SstatResult {
            avg_cpu: format_hms(cpu_secs),
            max_rss: format!("{}K", mem_mb * 1024),
            max_vmsize: format!("{}K", (mem_mb + 2000) * 1024),
        })
    }

    fn get_storage(&self) -> Vec<StorageInfo> {
        vec![
            StorageInfo {
                filesystem: "/dev/sda1".into(),
                size: "500G".into(),
                used: "312G".into(),
                avail: "188G".into(),
                use_pct: "62%".into(),
                mount: "/".into(),
            },
            StorageInfo {
                filesystem: "nfs-server:/home".into(),
                size: "50T".into(),
                used: "32T".into(),
                avail: "18T".into(),
                use_pct: "64%".into(),
                mount: "/home".into(),
            },
            StorageInfo {
                filesystem: "nfs-server:/scratch".into(),
                size: "200T".into(),
                used: "148T".into(),
                avail: "52T".into(),
                use_pct: "74%".into(),
                mount: "/scratch".into(),
            },
            StorageInfo {
                filesystem: "lustre:/data".into(),
                size: "1.0P".into(),
                used: "680T".into(),
                avail: "320T".into(),
                use_pct: "68%".into(),
                mount: "/data".into(),
            },
        ]
    }

    fn get_gpu_utilization(&self, _node: Option<&str>) -> Vec<f64> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    //! Contract tests: the mock must produce what the real parsers accept,
    //! so demo mode exercises the same code paths as a cluster.
    use super::*;
    use crate::slurm_api::{extract_form_state, resolve_log_path};

    #[test]
    fn queue_round_trips_through_the_squeue_parser() {
        let m = MockSlurmController::new(6, Some(7));
        let jobs = m.get_queue(None);
        assert!(!jobs.is_empty());
        for j in &jobs {
            assert!(!j.job_id.is_empty());
            assert!(!j.time_used.is_empty(), "job {} has no time", j.job_id);
            assert!(!j.work_dir.is_empty());
        }
        // The demo includes a job array, shown with squeue-style IDs
        assert!(jobs.iter().any(|j| j.display_id().contains("_[")));
        assert!(jobs.iter().any(|j| !j.array_task_id.is_empty()));
    }

    #[test]
    fn job_details_use_modern_fields_and_resolve_logs() {
        let m = MockSlurmController::new(4, Some(3));
        let job = m.get_queue(None).into_iter().next().unwrap();
        let d = m.get_job_details(&job.job_id).unwrap();
        assert!(d.get("current_working_directory").is_some());
        assert!(d.get("memory_per_node").is_some());
        let path = resolve_log_path(&d, false).unwrap();
        assert!(std::path::Path::new(&path).exists(), "{path}");
        let form = extract_form_state(&d);
        assert!(form.contains_key("memory"));
    }

    #[test]
    fn finished_jobs_come_from_accounting() {
        let m = MockSlurmController::new(0, Some(3));
        let row = m.get_sacct(None, None).into_iter().next().unwrap();
        let d = m.get_job_details(&row.job_id).unwrap();
        assert_eq!(d["slurmterm_source"], "sacct");
        assert_eq!(crate::slurm_api::job_state_of(&d), row.state);
        let path = resolve_log_path(&d, false).unwrap();
        assert!(std::path::Path::new(&path).exists(), "{path}");
    }

    #[test]
    fn history_is_stable_between_polls() {
        let m = MockSlurmController::new(0, Some(3));
        let a: Vec<String> = m.get_sacct(None, None).iter().map(|r| r.state.clone()).collect();
        let b: Vec<String> = m.get_sacct(None, None).iter().map(|r| r.state.clone()).collect();
        assert_eq!(a, b);
    }

    #[test]
    fn cancelling_the_array_id_cancels_every_task() {
        let m = MockSlurmController::new(2, Some(5));
        let master = m
            .get_queue(None)
            .into_iter()
            .find(|j| !j.array_task_string.is_empty())
            .unwrap()
            .array_job_id;
        assert!(m.cancel_job(&master));
        let jobs = m.get_queue(None);
        assert!(jobs
            .iter()
            .filter(|j| j.array_job_id == master)
            .all(|j| j.state == "CANCELLED"));
    }

    #[test]
    fn requeue_and_signal() {
        let m = MockSlurmController::new(0, Some(5));
        let id = m.inner.borrow_mut().spawn_job(Some("RUNNING"));
        assert!(m.signal_job(&id, "USR1", true));
        assert!(!m.signal_job(&id, "BOGUS", false));
        assert!(m.requeue_job(&id));
        let j = m.get_queue(None).into_iter().find(|j| j.job_id == id).unwrap();
        assert_eq!(j.state, "PENDING");
        assert!(!m.requeue_job(&id), "pending jobs cannot be requeued");
    }
}
