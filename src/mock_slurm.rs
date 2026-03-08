use std::cell::RefCell;
use std::collections::HashMap;

use crate::slurm_api::*;

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
        };
        for _ in 0..num_jobs {
            inner.spawn_job(None);
        }
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
        let elapsed = if state == "RUNNING" { self.rng.range(0, 36000) } else { 0 };
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
        });
        job_id
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
            if self.cancelled.contains(&id) || self.held.contains(&id) {
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
                "COMPLETING" => {
                    if self.rng.chance(0.4) {
                        self.jobs[i].state = "COMPLETED".to_string();
                    }
                }
                "PENDING" => {
                    if age > 10.0 && self.rng.chance(0.15) {
                        self.jobs[i].state = "RUNNING".to_string();
                    }
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

    fn get_queue(&self, _user: Option<&str>) -> Vec<JobInfo> {
        let mut inner = self.inner.borrow_mut();
        inner.tick();
        inner.jobs
            .iter()
            .map(|j| JobInfo {
                job_id: j.id.clone(),
                name: j.name.clone(),
                partition: j.partition.clone(),
                state: j.state.clone(),
                time_used: {
                    let h = j.elapsed / 3600;
                    let m = (j.elapsed % 3600) / 60;
                    let s = j.elapsed % 60;
                    format!("{h:02}:{m:02}:{s:02}")
                },
                nodes: j.nodes.clone(),
                reason: j.reason.clone(),
                user: j.user.clone(),
                work_dir: format!("/home/{}/projects/{}", j.user, j.name),
                stdout_path: j.log_path.clone(),
                stderr_path: j.log_path.replace(".out", ".err"),
                submit_time: String::new(),
                node_list: j.node_list.clone(),
                extra: HashMap::new(),
            })
            .collect()
    }

    fn get_partitions(&self) -> Vec<String> {
        PARTITIONS.iter().map(|s| s.to_string()).collect()
    }

    fn get_job_details(&self, job_id: &str) -> Option<serde_json::Value> {
        let inner = self.inner.borrow();
        let j = inner.jobs.iter().find(|j| j.id == job_id)?;
        let profile = match j.partition.as_str() {
            "debug" => ("4", "16000", ""),
            "gpu" => ("8", "32000", "gpu:a100:1"),
            "bigmem" => ("32", "256000", ""),
            _ => ("16", "64000", ""),
        };

        let details = serde_json::json!({
            "job_id": j.id,
            "name": j.name,
            "job_state": j.state,
            "partition": j.partition,
            "user_name": j.user,
            "working_directory": format!("/home/{}/projects/{}", j.user, j.name),
            "nodes": j.node_list,
            "standard_output": j.log_path,
            "standard_error": j.log_path.replace(".out", ".err"),
            "time_limit": j.time_limit / 60,
            "run_time": j.elapsed,
            "cpus_per_task": profile.0,
            "minimum_memory_per_node": profile.1,
            "gres_detail": profile.2,
            "slurmterm_metrics": {
                "cpu": j.metrics.cpu,
                "mem": j.metrics.mem,
                "gpu": j.metrics.gpu,
            }
        });
        Some(details)
    }

    fn cancel_job(&self, job_id: &str) -> bool {
        let mut inner = self.inner.borrow_mut();
        if let Some(idx) = inner.find_job(job_id) {
            inner.cancelled.insert(job_id.to_string());
            inner.jobs[idx].state = "CANCELLED".to_string();
            true
        } else {
            false
        }
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

    fn submit_job(
        &self,
        _script_path: &str,
        _params: &HashMap<String, String>,
    ) -> Result<String, String> {
        let mut inner = self.inner.borrow_mut();
        let id = inner.spawn_job(Some("PENDING"));
        Ok(id)
    }

    fn submit_wrap(
        &self,
        _commands: &str,
        _params: &HashMap<String, String>,
    ) -> Result<String, String> {
        let mut inner = self.inner.borrow_mut();
        let id = inner.spawn_job(Some("PENDING"));
        Ok(id)
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
        let mut inner = self.inner.borrow_mut();
        let mut rows = Vec::new();
        for i in 0..15 {
            let jid = format!("{}", 99900 + i);
            let elapsed_s = inner.rng.range(120, 86400);
            let h = elapsed_s / 3600;
            let m = (elapsed_s % 3600) / 60;
            let s = elapsed_s % 60;
            let elapsed = format!("{h:02}:{m:02}:{s:02}");
            let state_choices = &[
                "COMPLETED", "COMPLETED", "COMPLETED", "COMPLETED", "COMPLETED", "COMPLETED",
                "FAILED", "FAILED", "TIMEOUT", "CANCELLED",
            ];
            let state = inner.rng.choice(state_choices).to_string();
            let exit_code = if state == "COMPLETED" {
                "0:0".to_string()
            } else {
                format!("{}:0", inner.rng.range(1, 128))
            };
            let max_rss = format!("{}M", inner.rng.range(500, 64000));
            rows.push(SacctRow {
                job_id: jid,
                name: inner.rng.choice(JOB_NAMES).to_string(),
                partition: inner.rng.choice(PARTITIONS).to_string(),
                state,
                elapsed,
                total_cpu: "01:23:45".into(),
                max_rss,
                exit_code,
            });
        }
        rows
    }

    fn get_sstat(&self, job_id: &str) -> Option<SstatResult> {
        let inner = self.inner.borrow();
        let j = inner.jobs.iter().find(|j| j.id == job_id)?;
        if j.state != "RUNNING" {
            return None;
        }
        let cpu_pct = j.metrics.cpu.last().copied().unwrap_or(50.0);
        let mem_mb = (j.metrics.mem.last().copied().unwrap_or(50.0) / 100.0 * 32000.0) as i64;
        Some(SstatResult {
            avg_cpu: format!("{cpu_pct:.0}%"),
            max_rss: format!("{mem_mb}M"),
            max_vmsize: format!("{}M", mem_mb + 2000),
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
