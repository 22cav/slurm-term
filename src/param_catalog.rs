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
            Acceptable formats (sbatch(1)):\n\
            - minutes            (e.g. 120)\n\
            - MM:SS              (e.g. 30:00)\n\
            - HH:MM:SS           (e.g. 02:00:00)\n\
            - D-HH               (e.g. 2-12)\n\
            - D-HH:MM            (e.g. 2-12:30)\n\
            - D-HH:MM:SS         (e.g. 1-12:00:00)\n\
            - UNLIMITED / INFINITE  (partition MaxTime applies)\n\n\
            A limit of 0 also means no limit.\n\n\
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
            --mem=0 requests all memory on each node (the whole node's\n\
            memory is allocated, not \"no limit\").\n\
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
            Format: name[[:type]:count][,name...] -- count defaults to 1.\n\
            The count is per node.\n\n\
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
        long_desc: "Requests a specific number of GPUs on each node, like\n\
            --gres=gpu:<count>. Do not combine the two for GPUs.\n\n\
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
            Also: [a*2&b*1] per-node counts, [a|b] all nodes the same\n\
            feature. Use 'sinfo -o \"%N %f\"' to list node features.\n\n\
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
            Optional values: --exclusive=user (share only with your own\n\
            jobs), =mcs (share with jobs of the same MCS label),\n\
            =topo (exclusive topology block/segment).\n\
            With no value this is a flag.",
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
            - NONE            no emails\n\
            - BEGIN           job starts\n\
            - END             job completes\n\
            - FAIL            job fails\n\
            - REQUEUE         job is requeued\n\
            - ALL             BEGIN, END, FAIL, INVALID_DEPEND, REQUEUE, STAGE_OUT\n\
            - INVALID_DEPEND  dependency can never be satisfied\n\
            - STAGE_OUT       burst buffer stage out completed\n\
            - TIME_LIMIT      reached the time limit\n\
            - TIME_LIMIT_50/80/90  reached 50/80/90% of the limit\n\
            - ARRAY_TASKS     mail per array task, not per array\n\n\
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
            - after:JOBID[+min]  begin after JOBID starts (or is cancelled),\n\
                                 optionally +min minutes later\n\
            - afterok:JOBID      begin after JOBID succeeds (exit 0)\n\
            - afternotok:JOBID   begin after JOBID fails\n\
            - afterany:JOBID     begin after JOBID ends in any way\n\
            - aftercorr:JOBID    array task N starts after task N of JOBID succeeds\n\
            - afterburstbuffer:JOBID  after JOBID ends and its burst\n\
                                 buffer stage-out completes\n\
            - singleton          one running job per name and user\n\n\
            Several jobs: afterok:1:2. Several conditions: ',' (all\n\
            must hold) or '?' (any one).\n\n\
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
            - NONE   do not propagate the submission environment; the\n\
                     job starts with only SLURM_* variables\n\
            - ALL,VAR=val  everything plus overrides\n\
            - VAR1,VAR2=val  only these (and SLURM_*) variables\n\n\
            Note: with NONE, srun steps inside the script inherit the\n\
            job's environment, not yours.\n\n\
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
            - HH:MM[:SS]           (today, or tomorrow if past)\n\
            - YYYY-MM-DD[THH:MM[:SS]]  (absolute timestamp)\n\
            - now+N[seconds|minutes|hours|days|weeks]\n\
                                   (bare N is seconds)\n\
            - midnight, noon, fika (3 PM), teatime (4 PM),\n\
              today, tomorrow\n\n\
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
            Range: -2147483645 to 2147483645. Given without a value,\n\
            --nice lowers priority by 100. Only privileged users may\n\
            use negative values.\n\n\
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
        short_desc: "Allow job requeue",
        long_desc: "Permits the job to be requeued after a node failure or\n\
            preemption, or by 'scontrol requeue'. The job restarts from\n\
            the beginning; the default comes from JobRequeue in\n\
            slurm.conf. $SLURM_RESTART_COUNT tells the script how often\n\
            it was restarted.\n\n\
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
            Format: [{R|B}:]<sig_num|sig_name>[@sig_time]\n\
              B:  signal only the batch shell (not other steps)\n\
              R:  allow the job to overlap a reservation; the signal\n\
                  is sent before the reservation starts\n\
              sig_time defaults to 60 seconds. Delivery may be up\n\
              to 60 seconds early due to event resolution.\n\n\
            Example: --signal=USR1@120   (SIGUSR1, 2 min before end)",
        is_flag: false,
    },
    // Misc
    ParamEntry {
        key: "tmp",
        label: "Tmp Disk",
        short_desc: "Minimum temporary disk space per node",
        long_desc: "Requests that each allocated node has at least this much\n\
            temporary disk space (TmpFS) available. Default unit is\n\
            megabytes; suffixes K, M, G, T are accepted.\n\n\
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
        short_desc: "OCI container bundle path (native Slurm)",
        long_desc: "Absolute path to an unpacked OCI container bundle the job\n\
            runs inside. Requires oci.conf to be configured by the\n\
            administrator (Slurm's native container support).\n\n\
            Not the same as Pyxis's --container-image, which pulls an\n\
            image such as docker://nvcr.io/nvidia/pytorch.\n\n\
            Example: --container=/scratch/bundles/pytorch",
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
            - compute_bound   use all cores in each socket, one thread\n\
                              per core\n\
            - memory_bound    use only one core in each socket, one\n\
                              thread per core\n\
            - multithread     use extra threads (in-core multithreading)\n\
            - nomultithread   do not use extra threads\n\n\
            Incompatible with --ntasks-per-core, --threads-per-core\n\
            and -B.\n\n\
            Example: --hint=nomultithread",
        is_flag: false,
    },
    ParamEntry {
        key: "open-mode",
        label: "Open Mode",
        short_desc: "Output file open mode",
        long_desc: "Controls how output/error files are opened:\n\n\
            - append    append to existing file\n\
            - truncate  overwrite existing file\n\n\
            The default comes from JobFileAppend in slurm.conf\n\
            (truncate unless the administrator changed it). Requeued\n\
            jobs always append.\n\n\
            Example: --open-mode=append",
        is_flag: false,
    },
    ParamEntry {
        key: "distribution",
        label: "Distribution",
        short_desc: "Task distribution method",
        long_desc: "Controls how tasks are distributed across nodes, sockets,\n\
            and cores.\n\n\
            Format: <node>[:<socket>[:<core>]][,Pack|NoPack]\n\n\
            Node-level values:\n\
            - block      consecutive tasks share a node\n\
            - cyclic     round-robin across nodes\n\
            - plane=N    blocks of N tasks, round-robin\n\
            - arbitrary  follow the order in SLURM_HOSTFILE\n\n\
            Socket/core levels take block, cyclic or fcyclic.\n\n\
            Example: --distribution=cyclic",
        is_flag: false,
    },
    ParamEntry {
        key: "test-only",
        label: "Test Only",
        short_desc: "Validate without submitting",
        long_desc: "Validates the batch script and returns an estimate of when\n\
            the job would start, given the current queue and the rest\n\
            of the request. No job is submitted.\n\n\
            slurm-term shows the estimate in the status bar.\n\n\
            No value needed -- this is a flag.",
        is_flag: true,
    },
    // GPU / binding (sbatch(1))
    ParamEntry {
        key: "gpus",
        label: "GPUs (job)",
        short_desc: "Total GPUs for the whole job",
        long_desc: "Total number of GPUs for the job across all nodes, not\n\
            per node (compare --gpus-per-node / --gres).\n\n\
            Format: [type:]count. Short option: -G.\n\n\
            Example: --gpus=a100:8",
        is_flag: false,
    },
    ParamEntry {
        key: "ntasks-per-gpu",
        label: "Tasks / GPU",
        short_desc: "Tasks launched per allocated GPU",
        long_desc: "Request that there are this many tasks invoked for every\n\
            GPU. Implies --gpu-bind=single:<ntasks-per-gpu>.\n\n\
            Example: --ntasks-per-gpu=2",
        is_flag: false,
    },
    ParamEntry {
        key: "gpu-bind",
        label: "GPU Bind",
        short_desc: "Bind tasks to specific GPUs",
        long_desc: "Binds tasks to GPUs. Common values:\n\
            - closest          GPUs closest to the task's CPUs\n\
            - single:N         bind N tasks to each GPU\n\
            - map_gpu:0,1      explicit GPU per task\n\
            - none             no binding\n\n\
            Example: --gpu-bind=closest",
        is_flag: false,
    },
    ParamEntry {
        key: "mem-bind",
        label: "Mem Bind",
        short_desc: "NUMA memory binding",
        long_desc: "Binds tasks to NUMA memory: local, rank, map_mem:<list>,\n\
            mask_mem:<list>, none. Prefix with 'verbose,' to report.\n\n\
            Example: --mem-bind=local",
        is_flag: false,
    },
    // Scheduling
    ParamEntry {
        key: "time-min",
        label: "Min Time",
        short_desc: "Minimum acceptable time limit",
        long_desc: "Lets the scheduler start the job earlier with a shorter\n\
            time limit, no lower than this value (same formats as\n\
            --time). Useful for backfill.\n\n\
            Example: --time-min=02:00:00",
        is_flag: false,
    },
    ParamEntry {
        key: "deadline",
        label: "Deadline",
        short_desc: "Remove the job if it cannot finish by then",
        long_desc: "The job is removed if it cannot end before this time.\n\
            Formats like --begin (YYYY-MM-DD[THH:MM[:SS]], now+N...).\n\n\
            Example: --deadline=now+2days",
        is_flag: false,
    },
    ParamEntry {
        key: "hold",
        label: "Hold",
        short_desc: "Submit in held state",
        long_desc: "Submits the job with priority 0 (held). Release it with\n\
            'scontrol release <jobid>' (or 'u' in the Jobs tab).\n\n\
            No value needed -- this is a flag. Short option: -H.",
        is_flag: true,
    },
    ParamEntry {
        key: "kill-on-invalid-dep",
        label: "Kill Bad Dep",
        short_desc: "Cancel if dependency can never be met",
        long_desc: "yes: cancel the job when its dependency can never be\n\
            satisfied; no: leave it pending (DependencyNeverSatisfied).\n\n\
            Example: --kill-on-invalid-dep=yes",
        is_flag: false,
    },
    ParamEntry {
        key: "oversubscribe",
        label: "Oversubscribe",
        short_desc: "Share resources with other jobs",
        long_desc: "Allows the job's resources to be shared with other running\n\
            jobs, if the partition's OverSubscribe setting permits it.\n\n\
            No value needed -- this is a flag. Short option: -s.",
        is_flag: true,
    },
    ParamEntry {
        key: "switches",
        label: "Switches",
        short_desc: "Max leaf switches for the allocation",
        long_desc: "Maximum count of leaf switches desired for the job\n\
            allocation, optionally with a maximum wait for that many.\n\n\
            Format: count[@max-time]\n\n\
            Example: --switches=1@1:00:00",
        is_flag: false,
    },
    ParamEntry {
        key: "wckey",
        label: "WCKey",
        short_desc: "Workload characterization key",
        long_desc: "Tags the job with a workload characterization key used for\n\
            accounting reports (sreport).\n\n\
            Example: --wckey=projectX",
        is_flag: false,
    },
    ParamEntry {
        key: "clusters",
        label: "Clusters",
        short_desc: "Submit to another cluster",
        long_desc: "Cluster(s) to submit to in a multi-cluster setup. With\n\
            several names the job runs on the one that can start it\n\
            earliest. Short option: -M.\n\n\
            Example: --clusters=cluster2",
        is_flag: false,
    },
    ParamEntry {
        key: "wait",
        label: "Wait",
        short_desc: "sbatch returns only when the job ends",
        long_desc: "Do not exit until the submitted job terminates. The exit\n\
            code of sbatch will be that of the job.\n\n\
            Note: this blocks slurm-term until the job finishes or the\n\
            subprocess timeout expires -- rarely what you want here.\n\n\
            No value needed -- this is a flag. Short option: -W.",
        is_flag: true,
    },
    ParamEntry {
        key: "container-image",
        label: "Container Image",
        short_desc: "Container image (Pyxis plugin, not core Slurm)",
        long_desc: "Provided by the NVIDIA Pyxis SPANK plugin, when your site\n\
            installs it -- not part of core Slurm. Pulls and runs the job\n\
            in the given image.\n\n\
            Example: --container-image=nvcr.io#nvidia/pytorch:24.01-py3",
        is_flag: false,
    },
];
