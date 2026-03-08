/// Slurm parameter catalog with documentation.
///
/// Each entry provides a key, label, short description, and detailed help text.
/// Descriptions are based on the official Slurm documentation.
pub struct ParamEntry {
    pub key: &'static str,
    pub label: &'static str,
    pub short_desc: &'static str,
    pub long_desc: &'static str,
    pub is_flag: bool,
}

/// Core parameter keys that are always visible in the Composer form.
pub const CORE_PARAM_KEYS: &[&str] = &[
    "partition", "time", "nodes", "ntasks-per-node", "cpus-per-task",
    "mem", "gres", "job-name", "output", "error",
];

/// Map a Composer form field key to its param catalog key.
pub fn form_key_to_param(form_key: &str) -> &str {
    match form_key {
        "partition" => "partition",
        "time" => "time",
        "nodes" => "nodes",
        "ntasks" => "ntasks-per-node",
        "cpus" => "cpus-per-task",
        "memory" => "mem",
        "gpus" => "gres",
        "name" => "job-name",
        "output" => "output",
        "error" => "error",
        "script" => "_script-path",
        "modules" => "_modules",
        "env" => "_env-vars",
        "init" => "_init-cmds",
        "mode" => "_mode",
        other => other,
    }
}

pub fn lookup(key: &str) -> Option<&'static ParamEntry> {
    ALL_PARAMS.iter().chain(APP_FIELDS.iter()).find(|p| p.key == key)
}

pub static APP_FIELDS: &[ParamEntry] = &[
    ParamEntry {
        key: "_mode",
        label: "Mode",
        short_desc: "sbatch (batch) or srun (interactive)",
        long_desc: "Choose between batch (sbatch) and interactive (srun) submission.\n\n\
            sbatch: Submits a batch script for later execution. The script\n\
            runs unattended and output is captured to files.\n\n\
            srun: Launches an interactive session on allocated resources.\n\
            Useful for debugging, testing, and interactive work.",
        is_flag: false,
    },
    ParamEntry {
        key: "_script-path",
        label: "Script Path",
        short_desc: "Path to the batch script file",
        long_desc: "The filesystem path to the shell script that Slurm will execute\n\
            as your batch job. The script must be executable and should begin\n\
            with a shebang line (e.g. #!/bin/bash).\n\n\
            The script receives the environment and SBATCH directives from\n\
            this form. Any #SBATCH lines inside the script are overridden\n\
            by the settings you configure here.\n\n\
            Example: /home/user/train.sh",
        is_flag: false,
    },
    ParamEntry {
        key: "_modules",
        label: "Module Loads",
        short_desc: "Software modules to load before the job runs",
        long_desc: "Enter one module name per line. Each line becomes a\n\
            'module load <name>' command inserted at the top of your\n\
            batch script, before any user commands.\n\n\
            Modules configure PATH, LD_LIBRARY_PATH, and other environment\n\
            variables for specific software packages (CUDA, GCC, Python, etc.).\n\
            Use 'module avail' on the cluster to list available modules.\n\n\
            Examples:\n  cuda/12.1\n  python/3.11\n  gcc/13.1.0",
        is_flag: false,
    },
    ParamEntry {
        key: "_env-vars",
        label: "Environment Variables",
        short_desc: "Custom environment variables set before execution",
        long_desc: "Enter one KEY=VALUE pair per line. Each line becomes an\n\
            'export KEY=VALUE' statement in the batch script.\n\n\
            Use this to set application-specific configuration, paths,\n\
            or runtime flags that your script depends on.\n\n\
            Examples:\n  WANDB_PROJECT=my_experiment\n  OMP_NUM_THREADS=8\n  CUDA_VISIBLE_DEVICES=0,1",
        is_flag: false,
    },
    ParamEntry {
        key: "_init-cmds",
        label: "Init Commands",
        short_desc: "Shell commands executed before the main script",
        long_desc: "Enter arbitrary shell commands, one per line. These are inserted\n\
            into the batch script after module loads and environment variables\n\
            but before the main script path.\n\n\
            Common uses:\n\
            - Activate a conda/venv environment\n\
            - Create output directories\n\
            - Print diagnostic information\n\n\
            Examples:\n  source ~/venvs/torch/bin/activate\n  mkdir -p $SLURM_SUBMIT_DIR/results\n  echo \"Running on $(hostname)\"",
        is_flag: false,
    },
];

pub static ALL_PARAMS: &[ParamEntry] = &[
    // Core resource params
    ParamEntry {
        key: "partition",
        label: "Partition",
        short_desc: "Partition / queue to run in",
        long_desc: "Specifies the partition (queue) in which the job runs.\n\
            Partitions group nodes by hardware type, time limits, or access\n\
            policy. Use 'sinfo' to list available partitions.\n\n\
            If not specified, the cluster's default partition is used.\n\n\
            Example: --partition=gpu",
        is_flag: false,
    },
    ParamEntry {
        key: "time",
        label: "Time Limit",
        short_desc: "Maximum wall-clock time for the job",
        long_desc: "Sets the maximum wall-clock time the job may run. If the job\n\
            exceeds this limit Slurm will send SIGTERM followed by SIGKILL.\n\n\
            Acceptable formats:\n\
            - minutes            (e.g. 120)\n\
            - MM:SS              (e.g. 30:00)\n\
            - HH:MM:SS           (e.g. 02:00:00)\n\
            - D-HH:MM:SS         (e.g. 1-12:00:00)\n\n\
            Tip: Setting an accurate time limit improves scheduling priority\n\
            because the scheduler can backfill shorter jobs.",
        is_flag: false,
    },
    ParamEntry {
        key: "nodes",
        label: "Nodes",
        short_desc: "Number of nodes to allocate",
        long_desc: "Request a specific number of compute nodes.\n\n\
            For single-node jobs set --nodes=1. For distributed MPI jobs\n\
            this determines how many machines participate.\n\n\
            Examples:\n  --nodes=1     (single node)\n  --nodes=4     (exactly 4 nodes)",
        is_flag: false,
    },
    ParamEntry {
        key: "ntasks-per-node",
        label: "Tasks / Node",
        short_desc: "Number of tasks launched per node",
        long_desc: "Controls how many task instances Slurm launches on each\n\
            allocated node. Total tasks = nodes x ntasks-per-node.\n\n\
            For MPI: set this to the number of MPI ranks per node.\n\
            For single-process jobs: leave at 1.\n\n\
            Example: --ntasks-per-node=4",
        is_flag: false,
    },
    ParamEntry {
        key: "cpus-per-task",
        label: "CPUs / Task",
        short_desc: "CPU cores allocated per task",
        long_desc: "Advises Slurm on how many CPU cores each task requires.\n\
            Essential for multi-threaded applications (OpenMP, PyTorch\n\
            DataLoader workers, etc.).\n\n\
            Without this flag Slurm allocates 1 core per task.\n\n\
            Tip: For GPU jobs, set this to the number of CPU data-loading\n\
            threads you plan to use per GPU.\n\n\
            Example: --cpus-per-task=8",
        is_flag: false,
    },
    ParamEntry {
        key: "mem",
        label: "Memory",
        short_desc: "Minimum memory per node",
        long_desc: "Specifies the minimum RAM required per node. Default unit is\n\
            megabytes; use suffixes K, M, G, T for other units.\n\n\
            --mem=0 grants all available memory on the node.\n\
            Mutually exclusive with --mem-per-cpu and --mem-per-gpu.\n\n\
            Examples:\n  --mem=4G     (4 gigabytes)\n  --mem=64G    (64 gigabytes)\n  --mem=0      (all available memory)",
        is_flag: false,
    },
    ParamEntry {
        key: "gres",
        label: "GRES / GPUs",
        short_desc: "Generic resources (GPUs, FPGAs, etc.)",
        long_desc: "Requests generic consumable resources per node. Most commonly\n\
            used for GPUs.\n\n\
            Format: name[:type[:count]] -- count defaults to 1.\n\n\
            In the form, just enter the number of GPUs. The 'gpu:' prefix\n\
            is added automatically.\n\n\
            Examples (form field):\n  1         -> --gres=gpu:1\n  a100:2    -> --gres=gpu:a100:2",
        is_flag: false,
    },
    ParamEntry {
        key: "job-name",
        label: "Job Name",
        short_desc: "Name shown in the queue",
        long_desc: "Assigns a human-readable name to the job. This name appears\n\
            in squeue/sacct output and is used in filename patterns:\n\
            - %x expands to the job name\n\n\
            Max 200 characters. Avoid spaces and special characters.\n\n\
            Example: --job-name=train_resnet50",
        is_flag: false,
    },
    ParamEntry {
        key: "output",
        label: "Stdout File",
        short_desc: "File path for standard output",
        long_desc: "Redirects the job's stdout to the named file.\n\n\
            Slurm filename patterns:\n\
            - %j  job ID\n\
            - %x  job name\n\
            - %A  array master job ID\n\
            - %a  array task ID\n\
            - %N  first allocated node name\n\n\
            Default: slurm-%j.out\n\n\
            Example: --output=logs/%x-%j.out",
        is_flag: false,
    },
    ParamEntry {
        key: "error",
        label: "Stderr File",
        short_desc: "File path for standard error",
        long_desc: "Redirects the job's stderr to the named file.\n\
            Same filename patterns as --output.\n\n\
            By default stderr merges into the stdout file.\n\n\
            Example: --error=logs/%x-%j.err",
        is_flag: false,
    },
    // Account / scheduling
    ParamEntry {
        key: "account",
        label: "Account",
        short_desc: "Charge job to this account",
        long_desc: "Specifies which project account to charge for consumed\n\
            resources. Required on clusters where users belong to multiple\n\
            projects or allocations.\n\n\
            Use 'sacctmgr show associations user=$USER' to list your\n\
            available accounts.\n\n\
            Example: --account=myproject",
        is_flag: false,
    },
    ParamEntry {
        key: "qos",
        label: "QOS",
        short_desc: "Quality of Service level",
        long_desc: "Selects the Quality of Service for the job. QOS can affect\n\
            scheduling priority, preemption policy, and resource limits.\n\n\
            Common values: normal, high, low, debug, gpu.\n\
            Use 'sacctmgr show qos' to list available QOS levels.\n\n\
            Example: --qos=high",
        is_flag: false,
    },
    // GPU-specific
    ParamEntry {
        key: "gpus-per-node",
        label: "GPUs / Node",
        short_desc: "Number of GPUs per allocated node",
        long_desc: "Requests a specific number of GPUs on each node. Functionally\n\
            equivalent to --gres=gpu:<count> but clearer for multi-node\n\
            GPU jobs.\n\n\
            Can optionally specify GPU type: --gpus-per-node=a100:2\n\n\
            Example: --gpus-per-node=4",
        is_flag: false,
    },
    ParamEntry {
        key: "gpus-per-task",
        label: "GPUs / Task",
        short_desc: "Number of GPUs per task",
        long_desc: "Specifies how many GPUs each task needs. Slurm sets\n\
            CUDA_VISIBLE_DEVICES automatically for each task.\n\n\
            Example: --gpus-per-task=1",
        is_flag: false,
    },
    ParamEntry {
        key: "mem-per-gpu",
        label: "Mem / GPU",
        short_desc: "System memory per GPU",
        long_desc: "Requests a specific amount of system RAM per GPU.\n\
            Note: this is CPU/system memory, not GPU VRAM.\n\
            Mutually exclusive with --mem and --mem-per-cpu.\n\n\
            Example: --mem-per-gpu=32G",
        is_flag: false,
    },
    ParamEntry {
        key: "cpus-per-gpu",
        label: "CPUs / GPU",
        short_desc: "CPU cores per GPU",
        long_desc: "Specifies the number of CPU cores allocated per GPU.\n\
            Useful for ensuring enough CPU resources for data loading.\n\n\
            Example: --cpus-per-gpu=8",
        is_flag: false,
    },
    // Node features & constraints
    ParamEntry {
        key: "constraint",
        label: "Constraint",
        short_desc: "Require specific node features",
        long_desc: "Requests nodes with specific features set by the administrator\n\
            (e.g. CPU architecture, GPU model, interconnect type).\n\n\
            Operators:\n  &  AND (both features required)\n  |  OR  (either acceptable)\n\n\
            Use 'sinfo -o \"%N %f\"' to list node features.\n\n\
            Examples:\n  --constraint=haswell\n  --constraint='gpu_a100|gpu_v100'",
        is_flag: false,
    },
    ParamEntry {
        key: "exclusive",
        label: "Exclusive",
        short_desc: "Exclusive node access",
        long_desc: "Requests exclusive access to all allocated nodes -- no other\n\
            jobs will share them.\n\n\
            Useful for:\n\
            - Benchmarking (no interference from other jobs)\n\
            - Applications needing all memory, cache, or I/O bandwidth\n\
            - GPU jobs that need all GPUs on a node\n\n\
            No value needed -- this is a flag.",
        is_flag: true,
    },
    // Notifications
    ParamEntry {
        key: "mail-user",
        label: "Mail User",
        short_desc: "Email for job notifications",
        long_desc: "Specifies the email address where Slurm sends job event\n\
            notifications. Must be combined with --mail-type.\n\n\
            Example: --mail-user=user@example.com",
        is_flag: false,
    },
    ParamEntry {
        key: "mail-type",
        label: "Mail Type",
        short_desc: "Events that trigger email",
        long_desc: "Selects which job events generate email notifications.\n\
            Multiple types can be comma-separated.\n\n\
            Values:\n\
            - NONE         no emails\n\
            - BEGIN        job starts\n\
            - END          job completes\n\
            - FAIL         job fails\n\
            - ALL          all events\n\
            - TIME_LIMIT   approaching time limit\n\n\
            Example: --mail-type=END,FAIL",
        is_flag: false,
    },
    // Job arrays & dependencies
    ParamEntry {
        key: "array",
        label: "Job Array",
        short_desc: "Submit a job array",
        long_desc: "Creates a job array -- a set of similar jobs sharing the same\n\
            script but each receiving a unique SLURM_ARRAY_TASK_ID.\n\n\
            Formats:\n\
            - 0-9         10 tasks, IDs 0 through 9\n\
            - 1,3,5,7     4 specific tasks\n\
            - 0-99%10     100 tasks, max 10 running concurrently\n\
            - 1-100:2     odd IDs: 1,3,5,...,99\n\n\
            Inside the script use $SLURM_ARRAY_TASK_ID to differentiate.\n\
            sbatch only -- not available with srun.",
        is_flag: false,
    },
    ParamEntry {
        key: "dependency",
        label: "Dependency",
        short_desc: "Defer start until conditions met",
        long_desc: "Prevents the job from starting until specified dependencies\n\
            on other jobs are satisfied.\n\n\
            Types:\n\
            - after:JOBID        begin after JOBID starts\n\
            - afterok:JOBID      begin after JOBID succeeds\n\
            - afternotok:JOBID   begin after JOBID fails\n\
            - afterany:JOBID     begin after JOBID finishes\n\
            - singleton          wait for same-name jobs\n\n\
            Example: --dependency=afterok:12345",
        is_flag: false,
    },
    // Working directory & environment
    ParamEntry {
        key: "chdir",
        label: "Work Dir",
        short_desc: "Set working directory",
        long_desc: "Changes the working directory of the batch script before\n\
            execution. Equivalent to 'cd' at the start of the script.\n\n\
            If not specified, the job runs in the directory where sbatch\n\
            was invoked ($SLURM_SUBMIT_DIR).\n\n\
            Example: --chdir=/home/user/project",
        is_flag: false,
    },
    ParamEntry {
        key: "export",
        label: "Export Env",
        short_desc: "Control environment variable propagation",
        long_desc: "Controls which environment variables are passed to the job.\n\n\
            - ALL    export everything (default)\n\
            - NONE   clean environment, only SLURM_* defined\n\
            - VAR1,VAR2=val  export specific variables only\n\n\
            Using NONE is recommended for reproducibility.\n\n\
            Example: --export=ALL",
        is_flag: false,
    },
    // Scheduling
    ParamEntry {
        key: "begin",
        label: "Deferred Start",
        short_desc: "Defer job until a specific time",
        long_desc: "Delays job eligibility until the specified time.\n\n\
            Formats:\n\
            - YYYY-MM-DDTHH:MM:SS  (absolute timestamp)\n\
            - now+Nminutes         (relative offset)\n\
            - now+Nhours / now+Ndays\n\
            - midnight, noon       (special keywords)\n\n\
            Example: --begin=now+2hours",
        is_flag: false,
    },
    ParamEntry {
        key: "reservation",
        label: "Reservation",
        short_desc: "Use a named reservation",
        long_desc: "Requests that the job run within a specific advance reservation\n\
            created by the cluster administrator.\n\n\
            Use 'scontrol show reservations' to list available reservations.\n\n\
            Example: --reservation=gpu_maintenance",
        is_flag: false,
    },
    ParamEntry {
        key: "nice",
        label: "Nice",
        short_desc: "Scheduling priority adjustment",
        long_desc: "Adjusts the job's scheduling priority. Positive values lower\n\
            priority; negative values raise it (may require admin privilege).\n\n\
            Range: -10000 to 10000. Default: 0.\n\n\
            Example: --nice=100  (lower priority, more polite)",
        is_flag: false,
    },
    // Node selection
    ParamEntry {
        key: "exclude",
        label: "Exclude Nodes",
        short_desc: "Exclude specific nodes",
        long_desc: "Prevents the job from running on the listed nodes. Useful to\n\
            avoid known-problematic or overloaded nodes.\n\n\
            Supports Slurm hostlist notation.\n\n\
            Example: --exclude=node[001-003],node010",
        is_flag: false,
    },
    ParamEntry {
        key: "nodelist",
        label: "Node List",
        short_desc: "Request specific nodes",
        long_desc: "Requests that the job be allocated on the specified nodes.\n\n\
            Supports Slurm hostlist notation.\n\n\
            Example: --nodelist=gpu[005-008]",
        is_flag: false,
    },
    // Memory alternatives
    ParamEntry {
        key: "mem-per-cpu",
        label: "Mem / CPU",
        short_desc: "Memory per CPU core",
        long_desc: "Specifies memory per allocated CPU core instead of per node.\n\
            Mutually exclusive with --mem and --mem-per-gpu.\n\
            Units: K, M, G, T.\n\n\
            Example: --mem-per-cpu=2G",
        is_flag: false,
    },
    ParamEntry {
        key: "ntasks",
        label: "Total Tasks",
        short_desc: "Total number of tasks across all nodes",
        long_desc: "Specifies the total number of task instances. Slurm distributes\n\
            them across the allocated nodes.\n\n\
            Alternative to using --nodes x --ntasks-per-node.\n\n\
            Example: --ntasks=16",
        is_flag: false,
    },
    // Requeue & signals
    ParamEntry {
        key: "requeue",
        label: "Requeue",
        short_desc: "Allow job requeue on failure",
        long_desc: "Permits the job to be requeued if a node fails or the job is\n\
            preempted. The job restarts from the beginning.\n\n\
            No value needed -- this is a flag.",
        is_flag: true,
    },
    ParamEntry {
        key: "no-requeue",
        label: "No Requeue",
        short_desc: "Prevent job requeue",
        long_desc: "Prevents the job from being requeued under any circumstance.\n\n\
            No value needed -- this is a flag.",
        is_flag: true,
    },
    ParamEntry {
        key: "signal",
        label: "Signal",
        short_desc: "Send signal before time limit",
        long_desc: "Sends a signal to the job a specified number of seconds before\n\
            the time limit expires, allowing graceful checkpoint/shutdown.\n\n\
            Format: [B:]signal_name@seconds_before_end\n\
              B: prefix sends to the batch step only\n\n\
            Example: --signal=USR1@120   (SIGUSR1, 2 min before end)",
        is_flag: false,
    },
    // Misc
    ParamEntry {
        key: "tmp",
        label: "Tmp Disk",
        short_desc: "Minimum /tmp disk space per node (MB)",
        long_desc: "Requests that each allocated node has at least this much\n\
            temporary disk space available (in megabytes).\n\n\
            Example: --tmp=10240  (10 GB of temp space)",
        is_flag: false,
    },
    ParamEntry {
        key: "comment",
        label: "Comment",
        short_desc: "Attach a comment to the job",
        long_desc: "Sets an arbitrary comment string on the job, visible in sacct\n\
            and scontrol output. Useful for tagging experiments.\n\n\
            Example: --comment='experiment_v3_lr0.001'",
        is_flag: false,
    },
    ParamEntry {
        key: "licenses",
        label: "Licenses",
        short_desc: "Required software licenses",
        long_desc: "Requests software licenses managed by Slurm. The job will\n\
            not start until the licenses are available.\n\n\
            Format: name[:count][,name[:count]]\n\n\
            Example: --licenses=matlab:1,stata:2",
        is_flag: false,
    },
    ParamEntry {
        key: "overcommit",
        label: "Overcommit",
        short_desc: "Allow CPU overcommit",
        long_desc: "Allows more tasks than physical CPU cores on each node.\n\n\
            No value needed -- this is a flag.",
        is_flag: true,
    },
    ParamEntry {
        key: "container",
        label: "Container",
        short_desc: "OCI container image",
        long_desc: "Specifies an OCI container image for the job. Requires\n\
            container support (e.g. Pyxis plugin) in the Slurm config.\n\n\
            Example: --container=docker://nvcr.io/nvidia/pytorch:latest",
        is_flag: false,
    },
    ParamEntry {
        key: "spread-job",
        label: "Spread Job",
        short_desc: "Spread tasks evenly across nodes",
        long_desc: "Distributes tasks as evenly as possible across the allocated\n\
            nodes, rather than packing onto fewer nodes.\n\n\
            No value needed -- this is a flag.",
        is_flag: true,
    },
    ParamEntry {
        key: "hint",
        label: "CPU Hint",
        short_desc: "CPU binding performance hint",
        long_desc: "Provides a hint about the application's compute characteristics\n\
            to optimize CPU and thread binding.\n\n\
            Values:\n\
            - compute_bound   use all cores, disable hyperthreads\n\
            - memory_bound    use 1 thread per core\n\
            - multithread     use all hardware threads (SMT)\n\
            - nomultithread   explicitly disable hyperthreads\n\n\
            Example: --hint=nomultithread",
        is_flag: false,
    },
    ParamEntry {
        key: "open-mode",
        label: "Open Mode",
        short_desc: "Output file open mode",
        long_desc: "Controls how output/error files are opened:\n\n\
            - append    append to existing file\n\
            - truncate  overwrite existing file (default)\n\n\
            Example: --open-mode=append",
        is_flag: false,
    },
    ParamEntry {
        key: "distribution",
        label: "Distribution",
        short_desc: "Task distribution method",
        long_desc: "Controls how tasks are distributed across nodes, sockets,\n\
            and cores.\n\n\
            Values:\n\
            - block   fill each node before moving to the next\n\
            - cyclic  round-robin across nodes\n\
            - plane=N distribute in planes of N tasks\n\n\
            Example: --distribution=cyclic",
        is_flag: false,
    },
    ParamEntry {
        key: "test-only",
        label: "Test Only",
        short_desc: "Validate without submitting",
        long_desc: "Tests the job submission without actually submitting it.\n\
            Reports estimated start time and whether the job would be\n\
            accepted.\n\n\
            No value needed -- this is a flag.",
        is_flag: true,
    },
];
