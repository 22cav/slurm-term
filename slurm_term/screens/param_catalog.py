"""Slurm parameter catalog with detailed documentation.

Every parameter the Composer can produce — both official Slurm flags and
application-level fields (modules, environment, etc.) — has an entry here
with a short description and a detailed multi-line help text.

Descriptions are sourced from the official Slurm documentation at
https://slurm.schedmd.com/sbatch.html and https://slurm.schedmd.com/srun.html.
"""

from __future__ import annotations

# --------------------------------------------------------------------------
# Types
# --------------------------------------------------------------------------

ParamEntry = tuple[str, str, str, str]
"""(key, label, short_desc, long_desc)"""

# --------------------------------------------------------------------------
# Flags that take no value
# --------------------------------------------------------------------------

FLAG_PARAMS: set[str] = {
    "exclusive", "requeue", "no-requeue", "overcommit", "spread-job",
    "oversubscribe", "parsable", "wait", "test-only",
}

# --------------------------------------------------------------------------
# Core params — always visible in the Composer form
# --------------------------------------------------------------------------

CORE_PARAM_KEYS: set[str] = {
    "partition", "time", "nodes", "ntasks-per-node", "cpus-per-task",
    "mem", "gres", "job-name", "output", "error",
}

# --------------------------------------------------------------------------
# App-level fields (not --flags but form sections users need help with)
# --------------------------------------------------------------------------

APP_FIELDS: list[ParamEntry] = [
    ("_script-path", "Script Path", "Path to the batch script file",
     "The filesystem path to the shell script that Slurm will execute "
     "as your batch job.  The script must be executable and should begin "
     "with a shebang line (e.g. #!/bin/bash).\n\n"
     "The script receives the environment and SBATCH directives from "
     "this form.  Any #SBATCH lines inside the script are overridden "
     "by the settings you configure here.\n\n"
     "Example: /home/user/train.sh"),

    ("_modules", "Module Loads", "Software modules to load before the job runs",
     "Enter one module name per line.  Each line becomes a "
     "'module load <name>' command inserted at the top of your "
     "batch script, before any user commands.\n\n"
     "Modules configure PATH, LD_LIBRARY_PATH, and other environment "
     "variables for specific software packages (CUDA, GCC, Python, etc.).  "
     "Use 'module avail' on the cluster to list available modules.\n\n"
     "Examples:\n"
     "  cuda/12.1\n"
     "  python/3.11\n"
     "  gcc/13.1.0"),

    ("_env-vars", "Environment Variables", "Custom environment variables set before execution",
     "Enter one KEY=VALUE pair per line.  Each line becomes an "
     "'export KEY=VALUE' statement in the batch script.\n\n"
     "Use this to set application-specific configuration, paths, "
     "or runtime flags that your script depends on.\n\n"
     "Examples:\n"
     "  WANDB_PROJECT=my_experiment\n"
     "  OMP_NUM_THREADS=8\n"
     "  CUDA_VISIBLE_DEVICES=0,1"),

    ("_init-cmds", "Init Commands", "Shell commands executed before the main script",
     "Enter arbitrary shell commands, one per line.  These are inserted "
     "into the batch script after module loads and environment variables "
     "but before the main script path.\n\n"
     "Common uses:\n"
     "  • Activate a conda/venv environment\n"
     "  • Create output directories\n"
     "  • Print diagnostic information\n\n"
     "Examples:\n"
     "  source ~/venvs/torch/bin/activate\n"
     "  mkdir -p $SLURM_SUBMIT_DIR/results\n"
     "  echo \"Running on $(hostname)\""),
]

# --------------------------------------------------------------------------
# Full Slurm parameter catalog
# --------------------------------------------------------------------------

ALL_PARAMS: list[ParamEntry] = [
    # ── Core resource params ──
    ("partition", "Partition", "Partition / queue to run in",
     "Specifies the partition (queue) in which the job runs.  "
     "Partitions group nodes by hardware type, time limits, or access "
     "policy.  Use 'sinfo' to list available partitions and their "
     "properties (node count, time limit, available features).\n\n"
     "If not specified, the cluster's default partition is used.\n\n"
     "Example: --partition=gpu\n"
     "Example: --partition=batch"),

    ("time", "Time Limit", "Maximum wall-clock time for the job",
     "Sets the maximum wall-clock time the job may run.  If the job "
     "exceeds this limit Slurm will send SIGTERM followed by SIGKILL.\n\n"
     "Acceptable formats:\n"
     "  • minutes            (e.g. 120)\n"
     "  • MM:SS              (e.g. 30:00)\n"
     "  • HH:MM:SS           (e.g. 02:00:00)\n"
     "  • D-HH:MM:SS         (e.g. 1-12:00:00)\n\n"
     "Tip: Setting an accurate (not overly generous) time limit "
     "improves your scheduling priority on most clusters because "
     "the scheduler can backfill shorter jobs."),

    ("nodes", "Nodes", "Number of nodes to allocate",
     "Request a specific number of compute nodes.  Can be a single "
     "value or a range (min-max).\n\n"
     "For single-node jobs set --nodes=1.  For distributed MPI jobs "
     "this determines how many machines participate.\n\n"
     "Examples:\n"
     "  --nodes=1        (single node)\n"
     "  --nodes=4        (exactly 4 nodes)\n"
     "  --nodes=2-8      (between 2 and 8 nodes)"),

    ("ntasks-per-node", "Tasks / Node", "Number of tasks launched per node",
     "Controls how many task instances Slurm launches on each "
     "allocated node.  Total tasks = nodes × ntasks-per-node.\n\n"
     "For MPI: set this to the number of MPI ranks per node.\n"
     "For single-process jobs: leave at 1.\n\n"
     "Example: --ntasks-per-node=4"),

    ("cpus-per-task", "CPUs / Task", "CPU cores allocated per task",
     "Advises Slurm on how many CPU cores each task requires.  "
     "Essential for multi-threaded applications (OpenMP, PyTorch "
     "DataLoader workers, etc.) where each task spawns multiple "
     "threads.\n\n"
     "Without this flag Slurm allocates 1 core per task.\n\n"
     "Tip: For GPU jobs, set this to the number of CPU data-loading "
     "threads you plan to use per GPU.\n\n"
     "Example: --cpus-per-task=8"),

    ("mem", "Memory", "Minimum memory per node",
     "Specifies the minimum RAM required per node.  Default unit is "
     "megabytes; use suffixes K, M, G, T for other units.\n\n"
     "  --mem=0  grants all available memory on the node.\n\n"
     "Mutually exclusive with --mem-per-cpu and --mem-per-gpu.\n\n"
     "Examples:\n"
     "  --mem=4G          (4 gigabytes)\n"
     "  --mem=64G         (64 gigabytes)\n"
     "  --mem=0           (all available memory)"),

    ("gres", "GRES / GPUs", "Generic resources (GPUs, FPGAs, etc.)",
     "Requests generic consumable resources per node.  Most commonly "
     "used for GPUs.\n\n"
     "Format: name[:type[:count]]  — count defaults to 1.\n\n"
     "In the Composer form, just enter the GPU spec without the "
     "'gpu:' prefix — it is added automatically.\n\n"
     "Examples (in the form field):\n"
     "  1              → --gres=gpu:1\n"
     "  a100:2         → --gres=gpu:a100:2\n"
     "  v100:4         → --gres=gpu:v100:4"),

    ("job-name", "Job Name", "Name shown in the queue",
     "Assigns a human-readable name to the job.  This name appears "
     "in squeue/sacct output and is used in filename patterns:\n"
     "  • %x — expands to the job name\n\n"
     "Max 200 characters.  Avoid spaces and special characters.\n\n"
     "Example: --job-name=train_resnet50"),

    ("output", "Stdout File", "File path for standard output",
     "Redirects the job's standard output (stdout) to the named file.\n\n"
     "Slurm filename patterns:\n"
     "  • %j — job ID\n"
     "  • %x — job name\n"
     "  • %A — array master job ID\n"
     "  • %a — array task ID\n"
     "  • %N — first allocated node name\n\n"
     "Default: slurm-%j.out\n\n"
     "Example: --output=logs/%x-%j.out"),

    ("error", "Stderr File", "File path for standard error",
     "Redirects the job's standard error (stderr) to the named file.  "
     "Same filename patterns as --output.\n\n"
     "By default stderr merges into the stdout file.  Set this "
     "separately to split stdout and stderr.\n\n"
     "Example: --error=logs/%x-%j.err"),

    # ── Account / scheduling ──
    ("account", "Account", "Charge job to this account",
     "Specifies which project account to charge for consumed "
     "resources.  Required on clusters where users belong to multiple "
     "projects or allocations.\n\n"
     "Use 'sacctmgr show associations user=$USER' to list your "
     "available accounts.\n\n"
     "Example: --account=myproject"),

    ("qos", "QOS", "Quality of Service level",
     "Selects the Quality of Service for the job.  QOS can affect "
     "scheduling priority, preemption policy, and resource limits "
     "(max wall time, max nodes, etc.).\n\n"
     "Common values: normal, high, low, debug, gpu.\n"
     "Use 'sacctmgr show qos' to list available QOS levels.\n\n"
     "Example: --qos=high"),

    # ── GPU-specific ──
    ("gpus-per-node", "GPUs / Node", "Number of GPUs per allocated node",
     "Requests a specific number of GPUs on each node.  Functionally "
     "equivalent to --gres=gpu:<count> but clearer when requesting "
     "multi-node GPU jobs.\n\n"
     "Can optionally specify GPU type: --gpus-per-node=a100:2\n\n"
     "Example: --gpus-per-node=4"),

    ("gpus-per-task", "GPUs / Task", "Number of GPUs per task",
     "Specifies how many GPUs each task needs.  Slurm sets "
     "CUDA_VISIBLE_DEVICES automatically for each task.\n\n"
     "Recommended for jobs structured around per-task GPU allocation.\n\n"
     "Example: --gpus-per-task=1"),

    ("mem-per-gpu", "Mem / GPU", "System memory per GPU",
     "Requests a specific amount of system RAM per GPU.  Note: this "
     "is CPU/system memory, not GPU VRAM.\n\n"
     "Mutually exclusive with --mem and --mem-per-cpu.\n\n"
     "Example: --mem-per-gpu=32G"),

    ("cpus-per-gpu", "CPUs / GPU", "CPU cores per GPU",
     "Specifies the number of CPU cores allocated per GPU.  Useful "
     "for ensuring enough CPU resources for data loading per GPU.\n\n"
     "Example: --cpus-per-gpu=8"),

    # ── Node features & constraints ──
    ("constraint", "Constraint", "Require specific node features",
     "Requests nodes with specific features set by the administrator "
     "(e.g. CPU architecture, GPU model, interconnect type).\n\n"
     "Operators:\n"
     "  &  — AND (both features required)\n"
     "  |  — OR  (either feature acceptable)\n\n"
     "Use 'sinfo -o \"%N %f\"' to list node features.\n\n"
     "Examples:\n"
     "  --constraint=haswell\n"
     "  --constraint='gpu_a100|gpu_v100'\n"
     "  --constraint='intel&avx512'"),

    ("exclusive", "Exclusive", "Exclusive node access",
     "Requests exclusive access to all allocated nodes — no other "
     "jobs will share them.\n\n"
     "Useful for:\n"
     "  • Benchmarking (no interference from other jobs)\n"
     "  • Applications needing all memory, cache, or I/O bandwidth\n"
     "  • GPU jobs that need all GPUs on a node\n\n"
     "No value needed — this is a flag."),

    # ── Notifications ──
    ("mail-user", "Mail User", "Email for job notifications",
     "Specifies the email address where Slurm sends job event "
     "notifications.  Must be combined with --mail-type.\n\n"
     "Example: --mail-user=user@example.com"),

    ("mail-type", "Mail Type", "Events that trigger email",
     "Selects which job events generate email notifications.  "
     "Multiple types can be comma-separated.\n\n"
     "Values:\n"
     "  NONE         — no emails\n"
     "  BEGIN        — job starts\n"
     "  END          — job completes\n"
     "  FAIL         — job fails\n"
     "  REQUEUE      — job is requeued\n"
     "  ALL          — all events\n"
     "  TIME_LIMIT   — approaching time limit\n"
     "  TIME_LIMIT_90/80/50 — percentage warnings\n"
     "  ARRAY_TASKS  — per-task notifications for arrays\n\n"
     "Example: --mail-type=END,FAIL"),

    # ── Job arrays & dependencies ──
    ("array", "Job Array", "Submit a job array",
     "Creates a job array — a set of similar jobs sharing the same "
     "script but each receiving a unique SLURM_ARRAY_TASK_ID.\n\n"
     "Formats:\n"
     "  0-9         — 10 tasks, IDs 0 through 9\n"
     "  1,3,5,7     — 4 specific tasks\n"
     "  0-99%10     — 100 tasks, max 10 running concurrently\n"
     "  1-100:2     — odd IDs: 1,3,5,...,99\n\n"
     "Inside the script use $SLURM_ARRAY_TASK_ID to differentiate.\n\n"
     "sbatch only — not available with srun."),

    ("dependency", "Dependency", "Defer start until conditions met",
     "Prevents the job from starting until specified dependencies on "
     "other jobs are satisfied.\n\n"
     "Types:\n"
     "  after:JOBID        — begin after JOBID starts\n"
     "  afterok:JOBID      — begin after JOBID succeeds (exit 0)\n"
     "  afternotok:JOBID   — begin after JOBID fails\n"
     "  afterany:JOBID     — begin after JOBID finishes (any status)\n"
     "  aftercorr:JOBID    — for arrays: task N starts after JOBID task N\n"
     "  singleton          — wait for same-name jobs from this user\n\n"
     "Multiple: --dependency=afterok:111:222,afterany:333\n\n"
     "Example: --dependency=afterok:12345"),

    # ── Working directory & environment ──
    ("chdir", "Work Dir", "Set working directory",
     "Changes the working directory of the batch script before "
     "execution.  Equivalent to 'cd' at the start of the script.\n\n"
     "If not specified, the job runs in the directory where sbatch "
     "was invoked ($SLURM_SUBMIT_DIR).\n\n"
     "Example: --chdir=/home/user/project"),

    ("export", "Export Env", "Control environment variable propagation",
     "Controls which environment variables are passed from the "
     "submission environment to the job.\n\n"
     "  ALL   — export everything (default)\n"
     "  NONE  — clean environment, only SLURM_* defined;\n"
     "          you must 'module load' inside the script\n"
     "  VAR1,VAR2=val — export specific variables only\n\n"
     "Using NONE is recommended for reproducibility.\n\n"
     "Example: --export=ALL\n"
     "Example: --export=NONE"),

    # ── Scheduling ──
    ("begin", "Deferred Start", "Defer job until a specific time",
     "Delays job eligibility until the specified time.\n\n"
     "Formats:\n"
     "  YYYY-MM-DDTHH:MM:SS      (absolute timestamp)\n"
     "  now+Nminutes              (relative offset)\n"
     "  now+Nhours\n"
     "  now+Ndays\n"
     "  midnight, noon, teatime   (special keywords)\n\n"
     "Example: --begin=now+2hours\n"
     "Example: --begin=2026-03-01T08:00:00"),

    ("reservation", "Reservation", "Use a named reservation",
     "Requests that the job run within a specific advance reservation "
     "created by the cluster administrator.\n\n"
     "Use 'scontrol show reservations' to list available reservations.\n\n"
     "Example: --reservation=gpu_maintenance"),

    ("nice", "Nice", "Scheduling priority adjustment",
     "Adjusts the job's scheduling priority.  Positive values lower "
     "priority; negative values raise it (may require admin privilege).\n\n"
     "Range: -10000 to 10000.  Default: 0.\n\n"
     "Example: --nice=100    (lower priority, more polite)\n"
     "Example: --nice=-50    (higher priority)"),

    # ── Node selection ──
    ("exclude", "Exclude Nodes", "Exclude specific nodes",
     "Prevents the job from running on the listed nodes.  Useful to "
     "avoid known-problematic or overloaded nodes.\n\n"
     "Supports Slurm hostlist notation.\n\n"
     "Example: --exclude=node[001-003],node010"),

    ("nodelist", "Node List", "Request specific nodes",
     "Requests that the job be allocated on the specified nodes.\n\n"
     "Supports Slurm hostlist notation.\n\n"
     "Example: --nodelist=gpu[005-008]"),

    # ── Memory alternatives ──
    ("mem-per-cpu", "Mem / CPU", "Memory per CPU core",
     "Specifies memory per allocated CPU core instead of per node.\n\n"
     "Mutually exclusive with --mem and --mem-per-gpu.\n"
     "Units: K, M, G, T.\n\n"
     "Example: --mem-per-cpu=2G"),

    ("ntasks", "Total Tasks", "Total number of tasks across all nodes",
     "Specifies the total number of task instances.  Slurm distributes "
     "them across the allocated nodes.\n\n"
     "Alternative to using --nodes × --ntasks-per-node.\n\n"
     "Example: --ntasks=16"),

    # ── Requeue & signals ──
    ("requeue", "Requeue", "Allow job requeue on failure",
     "Permits the job to be requeued if a node fails or the job is "
     "preempted.  The job restarts from the beginning.\n\n"
     "No value needed — this is a flag."),

    ("no-requeue", "No Requeue", "Prevent job requeue",
     "Prevents the job from being requeued under any circumstance.\n\n"
     "No value needed — this is a flag."),

    ("signal", "Signal", "Send signal before time limit",
     "Sends a signal to the job a specified number of seconds before "
     "the time limit expires, allowing graceful checkpoint/shutdown.\n\n"
     "Format: [B:]signal_name@seconds_before_end\n"
     "  B: prefix sends to the batch step only\n\n"
     "Example: --signal=USR1@120   (SIGUSR1, 2 min before end)\n"
     "Example: --signal=B:INT@60   (SIGINT to batch step, 1 min)"),

    # ── Misc ──
    ("tmp", "Tmp Disk", "Minimum /tmp disk space per node (MB)",
     "Requests that each allocated node has at least this much "
     "temporary disk space available (in megabytes).\n\n"
     "Example: --tmp=10240   (10 GB of temp space)"),

    ("comment", "Comment", "Attach a comment to the job",
     "Sets an arbitrary comment string on the job, visible in sacct "
     "and scontrol output.  Useful for tagging experiments.\n\n"
     "Example: --comment='experiment_v3_lr0.001'"),

    ("wckey", "WCKey", "Workload characterization key",
     "Associates a workload characterization key with the job for "
     "accounting and tracking purposes.\n\n"
     "Example: --wckey=project_alpha"),

    ("switches", "Switches", "Network topology constraint",
     "Limits the number of network switches between allocated nodes "
     "for better communication performance.\n\n"
     "Format: count[@max_wait_time]\n\n"
     "Example: --switches=1@00:10:00"),

    ("licenses", "Licenses", "Required software licenses",
     "Requests software licenses managed by Slurm.  The job will "
     "not start until the licenses are available.\n\n"
     "Format: name[:count][,name[:count]]\n\n"
     "Example: --licenses=matlab:1,stata:2"),

    ("overcommit", "Overcommit", "Allow CPU overcommit",
     "Allows more tasks than physical CPU cores on each node.\n\n"
     "No value needed — this is a flag."),

    ("container", "Container", "OCI container image",
     "Specifies an OCI container image for the job.  Requires "
     "container support (e.g. Pyxis plugin) in the Slurm config.\n\n"
     "Example: --container=docker://nvcr.io/nvidia/pytorch:latest"),

    ("spread-job", "Spread Job", "Spread tasks evenly across nodes",
     "Distributes tasks as evenly as possible across the allocated "
     "nodes, rather than packing onto fewer nodes.\n\n"
     "No value needed — this is a flag."),

    ("hint", "CPU Hint", "CPU binding performance hint",
     "Provides a hint about the application's compute characteristics "
     "to optimize CPU and thread binding.\n\n"
     "Values:\n"
     "  compute_bound   — use all cores, disable hyperthreads\n"
     "  memory_bound    — use 1 thread per core\n"
     "  multithread     — use all hardware threads (SMT)\n"
     "  nomultithread   — explicitly disable hyperthreads\n\n"
     "Example: --hint=nomultithread"),

    ("profile", "Profile", "Job profiling (HDF5)",
     "Enables collection of profiling data into HDF5 files.\n\n"
     "Values: all, none, energy, task, filesystem, network\n"
     "Multiple values comma-separated.\n\n"
     "Example: --profile=all"),

    ("open-mode", "Open Mode", "Output file open mode",
     "Controls how output/error files are opened:\n\n"
     "  append    — append to existing file\n"
     "  truncate  — overwrite existing file (default)\n\n"
     "Example: --open-mode=append"),

    ("wait-all-nodes", "Wait All Nodes", "Wait for all nodes to be ready",
     "Delays job launch until all allocated nodes are booted and "
     "ready to execute.  Useful for multi-node jobs where node boot "
     "times may vary.\n\n"
     "Values: 0 (don't wait, default) or 1 (wait).\n\n"
     "Example: --wait-all-nodes=1"),

    ("distribution", "Distribution", "Task distribution method",
     "Controls how tasks are distributed across nodes, sockets, "
     "and cores.\n\n"
     "Values:\n"
     "  block    — fill each node before moving to the next\n"
     "  cyclic   — round-robin across nodes\n"
     "  plane=N  — distribute in planes of N tasks\n\n"
     "Can combine: node:socket:core (e.g. block:cyclic:cyclic)\n\n"
     "Example: --distribution=cyclic"),

    ("propagate", "Propagate", "Propagate resource limits",
     "Controls which resource limits from the submission environment "
     "are propagated to the job.\n\n"
     "Values: ALL, NONE, or specific limits:\n"
     "  AS, CORE, CPU, DATA, FSIZE, MEMLOCK, NOFILE, NPROC, RSS, STACK\n\n"
     "Example: --propagate=NONE"),

    ("test-only", "Test Only", "Validate without submitting",
     "Tests the job submission without actually submitting it.  "
     "Reports estimated start time and whether the job would be "
     "accepted.\n\n"
     "No value needed — this is a flag."),

    ("verbose", "Verbose", "Increase output verbosity",
     "Increases the detail level of Slurm informational messages.  "
     "Can be specified multiple times for more detail.\n\n"
     "Example: --verbose"),
]

# --------------------------------------------------------------------------
# Lookup structures
# --------------------------------------------------------------------------

PARAM_BY_KEY: dict[str, ParamEntry] = {}
for _p in ALL_PARAMS:
    PARAM_BY_KEY[_p[0]] = _p
for _p in APP_FIELDS:
    PARAM_BY_KEY[_p[0]] = _p

EXTRA_PARAMS: list[ParamEntry] = [p for p in ALL_PARAMS if p[0] not in CORE_PARAM_KEYS]
