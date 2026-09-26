# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Fixed — alignment with the Slurm documentation
- **Empty "Time" column on real clusters**: the queue parser read `time.elapsed` and `working_directory`, which exist only in the `sacct --json` layout. `squeue --json` provides `start_time`/`suspend_time`/`pre_sus_time`; elapsed time is now computed the way squeue's `%M` does, and durations of a day or more print as `D-HH:MM:SS` like squeue. The work directory is read from `current_working_directory`.
- **Short `#SBATCH` options produced invalid commands**: an unmapped short option such as `#SBATCH -n 4` or `-A proj` was re-emitted as `--n=4` / `--A=proj`, which sbatch rejects. Every short option from sbatch(1) now resolves to its long name.
- **Flag directives were dropped**: `#SBATCH --exclusive`, `--requeue`, `-H` and other options without a value were not recognised when loading a script.
- **Attached short values were dropped**: `-N2`, `-t30` and `-c=8` are now parsed (getopt style). Trailing `# comments` after a directive are stripped and quoted values unquoted.
- **Directives after the first command were honoured**: sbatch stops reading `#SBATCH` lines at the first executable line; the parser now does the same and keeps the later lines verbatim in the script body.
- **`--gpus`/`-G` changed meaning when loaded**: this is a per-job GPU count but was folded into the per-node `--gres=gpu:N` field. It now stays `--gpus`. Multi-GRES requests such as `--gres=gpu:1,nvme:1` are also kept verbatim.
- **Logs not found for jobs using filename patterns**: the Inspector opened `standard_output` literally (e.g. `logs/%x-%j.out`). It now uses Slurm's `stdout_expanded` (24.05+) or expands `%j %J %A %a %x %u %N %%` (with zero padding) itself, resolves relative paths against the job's working directory, uses the `slurm-%j.out` default when unset, and shows stdout for stderr when stderr is not split (sbatch's default).
- **Memory and run time in the Inspector**: memory is read from `memory_per_node`, falling back to `memory_per_cpu`, and labelled "/ node" or "/ CPU". Run time is computed from start/suspend times, so CPU% from `sstat` is correct too.
- **Unset JSON values shown as `0`**: `{"set": false, ...}` envelopes (NO_VAL) now read as empty instead of their placeholder number.
- **`squeue -u` ignored with `--json`** on some Slurm versions: the queue is also filtered client-side by user.
- **Job ID parsing on submit**: sbatch is now run with `--parsable` (`jobid[;cluster]`) instead of taking the last word of the human-readable message.
- **`--time=UNLIMITED` / `INFINITE` rejected** by the Composer's validation, although sbatch accepts them.
- **Resubmitting a pending GPU job lost its GPUs**: `gres_detail` is only filled for running jobs; the GPU request is now read from `tres_per_node` (e.g. `gres/gpu:a100=2`) and keeps the requested type. `--mem-per-cpu`, account, QOS, dependency and constraint are carried over too.
- **Node CPU load from JSON was 100x too high**: `scontrol show nodes --json` reports `cpu_load` as load x 100; it is now scaled like the text output. The Cluster tab also shows allocated/total CPUs.
- **Parameter help corrected against sbatch(1)**:
  - `--nice` range (±2147483645)
  - `--container` (native OCI bundle path, not a docker URL; that is Pyxis's `--container-image`)
  - `--hint` definitions
  - all `--mail-type` values
  - dependency types (`aftercorr`, `afterburstbuffer`, `after:id+min`, the `?` separator)
  - `--signal` `R:` prefix
  - `--exclusive=user|mcs|topo`
  - `--time` day formats
  - `--begin` keywords
  - `--export=NONE` semantics
  - `--open-mode` default
  - `--distribution` levels

### Added
- **Inspect finished jobs**: jobs that slurmctld has already purged (after MinJobAge) are looked up with `sacct -j <id> --json`, so `Enter` in the History tab works for old jobs. The Inspector marks these as "from accounting (sacct)".
- **Job arrays in the Jobs tab**: IDs are shown as squeue prints them (`120_3`, `120_[4-9%2]`). `a` groups each array into one row with a state summary (`1PD 4R`); actions on that row apply to the whole array, and the confirmation says so.
- **Requeue (`R`)** a job with `scontrol requeue`, after a confirmation.
- **Signal (`K`)**: a picker sends USR1/USR2/TERM/INT/HUP/CONT/STOP/KILL with `scancel --signal`, optionally to the batch step only (`--batch`).
- **`--test-only` support**: the scheduler's start estimate is shown in the status bar and nothing is queued.
- More sbatch options in the parameter catalog: `--gpus`, `--ntasks-per-gpu`, `--gpu-bind`, `--mem-bind`, `--time-min`, `--deadline`, `--hold`, `--kill-on-invalid-dep`, `--oversubscribe`, `--switches`, `--wckey`, `--clusters`, `--wait`, plus Pyxis's `--container-image` (marked as a plugin option).
- Hold/release/requeue/signal report partial failures with Slurm's error message instead of always claiming success.

### Changed
- Demo mode now produces the real `squeue --json` / `scontrol show job --json` / `sacct --json` layouts and runs them through the same parsers a cluster uses. It includes a job array, and its history stays the same between polls.
- Slurm 23.02 or newer is the supported baseline; older JSON field names are still read where it is cheap to do so.

### Tests
- The suite grows from 55 to 103 tests. It adds JSON fixtures in `tests/fixtures/` for squeue 23.11, scontrol 24.05 and sacct, which cover elapsed time, arrays, user filtering, form extraction, log-path resolution and the sacct mapping.
- The sbatch parser gets its first tests: short options, flags, attached values, comments, directive scope, GPU semantics and CRLF input.
- Composer tests check that no invalid `--X` option is ever emitted, that the preview round-trips through the parser, and that every catalog key is valid.
- Monitor tests cover array grouping, action targets and the signal picker.
- Mock contract tests check that demo data satisfies the real parsers.

## [0.1.8]

### Fixed
- **Recovery from terminal-side screen clears (`⌘K`)**: `⌘K` in Terminal.app/iTerm2 clears the terminal's own buffer underneath the interface, which a diff-based renderer cannot detect — the display looked destroyed. The interface now repaints fully when the terminal regains focus, and `F5` forces a repaint + data refresh from any mode.
- **Editing keys now work in every field, not just the multiline editors**: single-line form fields (name, time, memory, …) ran on a leftover hand-rolled editor without `Ctrl+K`/`Ctrl+A`/`Ctrl+E`/undo. All text entry now runs through the same `tui-textarea` engine, with per-state help bars (the form-field editing state previously showed navigation hints, hiding how to exit).
- **Typed text can no longer be silently lost**: in-progress edits are committed on every exit path — `Esc`, `Enter`, `Tab`, mouse click, `Ctrl+S`, `Ctrl+T`. Clicking from a modified preview into the form now applies the preview edits instead of discarding them; clicking away from the Modules/Env/Init popup saves it.
- **`Esc` in the preview pane returns to the form**, making `Esc` a consistent "step out" key throughout the Composer.
- **The quit hint in the help bar is now state-aware**: while typing in any editor or search field, `q` inserts a character rather than quitting, so the bar shows `^C Quit` there and `q Quit` only when `q` actually quits.
- **macOS Command key mishandled**: on terminals that forward it, `⌘` arrives as the `SUPER` modifier, which the app didn't recognise — `⌘S` and friends did nothing and, inside the editor, leaked through as a typed character. `⌘` is now treated as an alias for `Control` everywhere, so `⌘`/`⌃` shortcuts are interchangeable. (Note: terminals such as Terminal.app that never forward `⌘` still require `Control`.)
- **Every keystroke could fire twice**: key events weren't filtered by kind, so on Windows and on terminals using the enhanced keyboard protocol the release event re-triggered each action. Only press/repeat events are handled now.
- **100% CPU while idle**: the event-loop timeout was derived from the multi-second data-poll timer, so once a render tick elapsed the loop busy-spun until the next poll. Input waits are now paced by a dedicated render tick, dropping idle CPU to near zero.
- **Module/env/init setup was silently dropped on submit**: when a Script Path was set, the generated `module load` / `export` / init lines shown in the preview were discarded and only the bare script was submitted. Submission now uses the previewed body whenever setup commands are present, so what you see is what runs.
- **Unknown `#SBATCH` directives were lost**: options without a dedicated form field (e.g. `--account`) were parsed but never re-emitted, so loading a script or editing the preview dropped them on submit. They are now preserved as visible extra-parameter rows and included in the submitted command.
- **Invalid `--gres` when resubmitting a GPU job**: `gres_detail` (e.g. `gpu:a100:2(IDX:0-1)`) was placed in the GPUs field verbatim and then re-prefixed with `gpu:`, producing `gpu:gpu:a100:2(...)`. Only the trailing GPU count is now extracted.
- **Mouse clicks selected the wrong row in scrolled tables**: click-to-select ignored the table's scroll offset in the Jobs, History and Cluster tabs, selecting a row above the cursor once the list scrolled. Extra-parameter rows in the Composer are now clickable too.
- **Possible crash editing multibyte text**: vertical cursor movement in the form, preview and field editors carried a byte column between lines, which could land mid-character and panic on the next edit. Cursor positions are now snapped to a char boundary.
- **Numeric and duration columns sorted lexically**: job IDs, elapsed/time columns, and CPU/node counts sorted as strings (`99999` after `100000`, `1-00:00:00` before `02:00:00`). They now sort by value.
- **Paste into an added extra parameter did nothing**: bracketed-paste only targeted core form fields.
- **Header and status bar misaligned**: right-aligned banner width was measured in bytes, so the `│` separators (and `—` in error messages) shifted the layout.
- **History window ignored the configured span**: a `history_window` that didn't match a preset (e.g. `now-5days`) highlighted the wrong selector entry; it now snaps to the nearest covering window and stays consistent.

### Changed
- **Consistent, safer keys in the Jobs tab**: `k` now moves the cursor up (matching every other tab) and job cancellation moved to `x`; the inspector no longer swallows `q` and the `1`–`4` tab switches. Command errors in the status bar are shown in red.
- **Composer editors now use the `tui-textarea` widget**: the script preview and the Modules/Env/Init popup previously ran ~700 lines of hand-rolled, duplicated editing logic. They now use the well-tested `tui-textarea` widget, which brings undo/redo (`Ctrl+U`/`Ctrl+R`), correct Unicode handling, and standard Emacs-style editing keys, and removes ~500 lines of bespoke editor code.
- **Composer key scheme made coherent with the editor**: copy-preview-to-clipboard moved from `Ctrl+Y` to `Ctrl+G` so it no longer shadows the editor's paste; the help bar now shows the correct undo key and a dedicated hint (with `Esc` to close) while the Modules/Env/Init popup is open.
- **Partition is no longer required to submit**: an empty partition now lets `sbatch` fall back to the cluster's default instead of blocking submission.

## [0.1.7]

### Fixed
- **Crash after idle in the job inspector**: viewing a job's logs and leaving the TUI idle would panic and dump an error to the terminal once the job finished. The log buffer shrinks when the job's log path disappears, but the scroll offset was left pointing past the end, producing an out-of-bounds slice on the next redraw. The offset is now clamped on every log reload and again at render time.
- **Crash in narrow terminals**: an unchecked subtraction in the header layout could underflow when the terminal was too narrow for the cluster/user/node banner.
- **Live CPU and memory sparklines never appeared on real clusters**: modern Slurm reports `job_state` as an array (`["RUNNING"]`), but the inspector compared it as a plain string, so the state never matched `RUNNING` and `sstat` was never called. All JSON state reads now go through a shared helper that accepts both shapes.
- **MaxRSS was always blank in History on real clusters**: `sacct` only populates `MaxRSS` on step rows (`123.batch`), which the parser discarded. Step rows are now folded into their parent job, carrying the maximum RSS across steps (unit-aware) and filling in `TotalCPU` when the job row omits it.
- **Memory reported in the wrong unit**: the Cluster tab labelled columns `Mem(GB)`/`Free(GB)` while printing raw mebibytes from `sinfo %m` and `scontrol` `RealMemory`/`FreeMem`. Values are now converted and the columns read `Mem(GiB)`/`Free(GiB)`.
- **Node and partition states were never colour-coded**: node states were matched in lowercase while `scontrol` reports them uppercase, and the partition `State` column was styled with `up`/`down` logic that belongs to the `Avail` column. Both now use a shared, case-insensitive mapping in which problem flags (`IDLE+DRAIN`) take precedence over the base state.
- **`scontrol show nodes` parsing corrupted several fields**: the key/value regex truncated values containing spaces or `=` (`OS=`, multi-word `Reason=`, `CfgTRES=cpu=16,mem=64G`). Node data is now read from `--json` where available, with a corrected tokenizer as fallback.
- **`--time` validation used the wrong unit**: a bare integer was treated as seconds, but `sbatch` interprets `--time=<n>` as minutes. The `days-hours` and `days-hours:minutes` forms were also mishandled.
- **Incomplete job-state colours**: added the documented states that fell through to grey (`BOOT_FAIL`, `DEADLINE`, `REVOKED`, `REQUEUED`, `RESIZING`, `SIGNALING`, `STAGE_OUT`, `SPECIAL_EXIT`, `CONFIGURING`), and states are now matched on their first word so `sacct`'s `CANCELLED by <uid>` is coloured correctly.
- **Storage view could mis-parse `df` output**: long device names wrap onto a second line with `df -h`, breaking the column parse. Now uses `df -hP`.

### Added
- **Interactive `srun` sessions actually launch `srun`**: submitting in `srun` mode previously built an interactive command in the preview but silently submitted a batch job instead. It now suspends the TUI, hands the terminal to `srun --pty … $SHELL`, and restores the interface when the session exits. `Ctrl+C` inside the session no longer kills slurm-term.
- **Slurm errors are surfaced in the status bar**: failing commands previously produced silently empty views. Failures now appear in the status line, including a specific message when the cluster's Slurm is too old for `--json` (requires 20.11+).
- **Command timeouts**: the configurable `subprocess_timeout` was accepted but never enforced, so a hung Slurm client could freeze the interface indefinitely. Commands are now killed at the deadline.
- **GPU utilisation sparkline**: the previously inert `[gpu]` configuration is now wired up. When `enabled = true`, the inspector samples GPU utilisation on the job's first node. Off by default.
- Test suite covering the log-scroll crash, Slurm output parsers, validators, and the command timeout.

### Changed
- Demo mode now emits the same JSON and `sacct`/`sstat` shapes as a real cluster, so it exercises the same code paths instead of masking format bugs.
- Internal cleanup: consolidated duplicated duration formatting, Slurm JSON unwrapping, and mouse hit-testing; regexes are now compiled once instead of on every call (previously recompiled on every keystroke in the Composer). Zero clippy warnings under `-D warnings`.

## [0.1.6]

### Added
- **File browser for script loading**: replaced the text-input file dialog with a full filesystem browser (`Ctrl+O`). Navigates directories, shows only compatible files (`.sbatch`, `.sh`, `.job`), and validates script headers before loading.
- **Full-featured text editor** in the preview pane: line numbers in gutter, word-level navigation (`Ctrl+Left/Right`), line operations (`Ctrl+K` kill line, `Ctrl+D` delete line, `Ctrl+U` clear to start), auto-indentation on Enter, and Tab/Shift+Tab for indent/dedent.
- **Multiline field editor**: Modules, Env Vars, and Init Cmds now open in the right pane with the same full editor when pressing Enter, instead of inline editing. Fields show a clean summary in the form (e.g., `numpy, scipy  [2 modules]`).
- **Native text selection in Inspector**: mouse capture is disabled when the inspector is open, allowing standard terminal drag-to-select and copy for job details and logs.

### Fixed
- **Log display corruption**: stripped ANSI escape sequences and handled carriage returns (`\r`) in log output, fixing lines overlaying each other in Follow mode.
- **N/A fields in job inspector**: correctly parse Slurm 22+ JSON object wrappers (`{"number": N, "set": bool, "infinite": bool}`) and string-encoded numbers from mock/demo mode for time, memory, and all scalar fields.
- **Form field highlight leak**: the focused form field no longer turns green when editing the preview pane; the green editing indicator now only appears on the active pane.

### Changed
- **Smarter log polling**: logs now refresh even when Follow is paused (at the main poll interval), and use file-size change detection to skip redundant re-reads when the file hasn't changed.
- **Cleaner multiline field display**: Modules, Env Vars, and Init Cmds show a compact comma-separated summary with item count instead of raw multiline text in the form.
- **Context-aware status bar**: the bottom hints now change between form-mode and editor-mode shortcuts when editing the preview pane.
- Resolved all clippy warnings.

## [0.1.5]

### Fixed
- **Fixed script submission breaking shell control structures**: jobs with `for`/`do`/`done` loops, line continuations, or multiline init commands no longer fail with syntax errors. Submission now writes a proper temp script file instead of collapsing everything into `sbatch --wrap` with `; ` joins.
- **Fixed CRLF line endings in submitted scripts**: `\r\n` line endings from pasted text or loaded files are now normalized to `\n` at all entry points (file loading, clipboard paste, parser).
- **Fixed multiline form fields disappearing**: Init Cmds, Modules, and Env Vars fields with many lines no longer overflow and vanish when focused. The display is now capped at 8 visible lines with scroll tracking and a line-count indicator.

### Added
- **Bracketed paste support**: pasting text into the Composer (both form fields and preview editor) now uses terminal bracketed paste mode, correctly handling multiline pastes with proper `\r` filtering.

### Changed
- **Faster log following**: Inspector log file refresh now runs on a separate 1-second timer (down from 3s) when follow mode is active, for near-real-time output tailing.
- Removed the `sbatch --wrap` submission path; all submissions now go through script files.

## [0.1.4]

### Fixed
- Fixed Linux wheel compatibility: build now targets `manylinux_2_17` (glibc 2.17+) for broader HPC cluster support.

## [0.1.3]

### Changed
- **Full rewrite in Rust** — native binary, no Python runtime or dependencies required.
- Instant startup (~2 ms vs ~1 s for the Python version).
- Single static binary installable via `pip install slurm-term` or `cargo install slurm-term`.

### Added
- **Cursor-based form editing** in Composer: arrow keys, Home/End, Delete navigate within field values.
- **`.sbatch` file loading** via `Ctrl+O` dialog or `--file` CLI argument.
- **40+ parameter catalog** with built-in documentation (`?` key) and searchable add-param dialog (`a` key).
- **Full multiline editing** for Modules, Env Vars, and Init Cmds fields with Up/Down line navigation.
- **Mouse support**: click to navigate tabs, select jobs, and interact with the form.
- **Input validation** with inline red highlighting for Time, Memory, Name, Nodes, Tasks, CPUs, GPUs fields.
- **Direct preview editing** with bidirectional sync to form fields.
- **Live log follow** in Inspector: logs auto-scroll with a `FOLLOW`/`PAUSED` indicator; press `f` to toggle, scrolling up pauses automatically.
- **Hostname awareness**: header bar shows node type (login/compute); hover to reveal full hostname.
- **Sortable tables**: press `s` to cycle sort column, `S` to reverse direction in Monitor, History, and Cluster tabs.
- **Storage display**: new Storage sub-tab in Cluster showing filesystem usage (`df -h`), with color-coded usage percentages.
- **Copy to clipboard**: `Ctrl+Y` in Composer copies the generated script to the system clipboard via OSC 52.

### Removed
- Python runtime dependency (textual, textual-plotext).
- Inspector is now inline in the Monitor tab (press `Enter` on a job) rather than a separate tab.

## [0.1.2]

### Added
- **Sbatch file import** (`Ctrl+I`): parse `.sbatch` scripts and populate the Composer form.
- **Quick-peek output** (`o`): preview the last 50 lines of a job's output from the Monitor.
- **Multi-select** (`Space` / `Ctrl+A`): bulk cancel, hold, or release multiple jobs.
- **Resubmit job** (`s`): re-populate the Composer from the Inspector or History tab.
- **History time window selector**: choose 1–30 day range directly in the History tab.
- **Improved interaction**: Error catching and improved design.

## [0.1.1]

Initial public release with Monitor, Composer, Hardware, History, and Inspector tabs, `--demo` mode, template save/load, parameter catalog, and inline validation.
