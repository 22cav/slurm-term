use std::collections::HashMap;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::app::centered_rect;
use crate::param_catalog;
use crate::sbatch_parser;
use crate::slurm_api::SlurmController;
use crate::templates;
use crate::theme;
use crate::validators::{parse_time, parse_memory, validate_job_name};

pub enum Action {
    None,
    Submit(HashMap<String, String>, String, String), // (params, script_path, wrap_commands)
    RunInteractive(Vec<String>), // full srun argv for a terminal handover
    Status(String),
}

#[derive(Clone, Copy, PartialEq)]
enum Field {
    Mode,
    Partition,
    Time,
    Nodes,
    Ntasks,
    Cpus,
    Memory,
    Gpus,
    Name,
    Script,
    Output,
    Error,
    Modules,
    Env,
    Init,
}

impl Field {
    fn all() -> &'static [Field] {
        &[
            Field::Mode, Field::Partition, Field::Time, Field::Nodes,
            Field::Ntasks, Field::Cpus, Field::Memory, Field::Gpus,
            Field::Name, Field::Script, Field::Output, Field::Error,
            Field::Modules, Field::Env, Field::Init,
        ]
    }

    fn label(&self) -> &'static str {
        match self {
            Field::Mode => "Mode",
            Field::Partition => "Partition",
            Field::Time => "Time Limit",
            Field::Nodes => "Nodes",
            Field::Ntasks => "Tasks/Node",
            Field::Cpus => "CPUs/Task",
            Field::Memory => "Memory",
            Field::Gpus => "GPUs",
            Field::Name => "Job Name",
            Field::Script => "Script Path",
            Field::Output => "Output",
            Field::Error => "Error",
            Field::Modules => "Modules",
            Field::Env => "Env Vars",
            Field::Init => "Init Cmds",
        }
    }

    fn is_sbatch_only(&self) -> bool {
        matches!(
            self,
            Field::Name | Field::Script | Field::Output | Field::Error
                | Field::Modules | Field::Env | Field::Init
        )
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Pane {
    Form,
    Preview,
}

pub struct ComposerState {
    pub fields: HashMap<String, String>,
    pub partitions: Vec<String>,
    pub editing: bool,
    pub active_pane: Pane,
    focus: usize,
    mode_is_srun: bool,
    pub template_dialog: Option<TemplateDialog>,
    // Editable preview
    pub preview_text: String,
    preview_cursor: usize,
    preview_scroll: usize,
    preview_dirty: bool, // true = preview text was edited manually
    // Cursor position within the currently edited form field
    field_cursor: usize,
    // Scroll offset for multiline form fields (line index of first visible line)
    field_scroll: usize,
    // Extra parameters added via catalog
    pub extra_params: Vec<(String, String)>, // (sbatch_key, value)
    // Help overlay
    pub help_overlay: bool,
    // Add parameter dialog
    pub add_param_dialog: Option<AddParamDialog>,
    // File browser dialog
    pub file_browser: Option<FileBrowserDialog>,
    // Multiline field editor: takes over the right pane for Modules/Env/Init
    field_editor: Option<FieldEditor>,
}

/// State for editing a multiline field in the right pane with full editor features.
struct FieldEditor {
    field: Field,
    text: String,
    cursor: usize,
    scroll: usize,
}

pub struct AddParamDialog {
    pub search: String,
    pub selected: usize,
}

pub enum TemplateDialog {
    Save { name: String },
    Load { names: Vec<String>, selected: usize },
}

// ---------------------------------------------------------------------------
// File browser dialog
// ---------------------------------------------------------------------------

const SCRIPT_EXTENSIONS: &[&str] = &["sbatch", "sh", "job"];

struct FileEntry {
    name: String,
    is_dir: bool,
    is_compatible: bool,
}

pub struct FileBrowserDialog {
    current_dir: PathBuf,
    entries: Vec<FileEntry>,
    selected: usize,
    scroll: usize,
    error: Option<String>,
}

impl FileBrowserDialog {
    fn new() -> Self {
        let dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
        let mut fb = Self {
            current_dir: dir,
            entries: Vec::new(),
            selected: 0,
            scroll: 0,
            error: None,
        };
        fb.refresh_entries();
        fb
    }

    fn refresh_entries(&mut self) {
        self.entries.clear();
        self.error = None;

        let read_dir = match std::fs::read_dir(&self.current_dir) {
            Ok(rd) => rd,
            Err(e) => {
                self.error = Some(format!("Cannot read directory: {e}"));
                return;
            }
        };

        let mut dirs: Vec<FileEntry> = Vec::new();
        let mut files: Vec<FileEntry> = Vec::new();

        for entry in read_dir.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue; // skip hidden files
            }
            let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);
            if is_dir {
                dirs.push(FileEntry { name, is_dir: true, is_compatible: false });
            } else {
                let has_ext = SCRIPT_EXTENSIONS.iter().any(|ext| {
                    name.ends_with(&format!(".{ext}"))
                });
                if has_ext {
                    let compatible = check_script_compatible(&entry.path());
                    files.push(FileEntry { name, is_dir: false, is_compatible: compatible });
                }
                // non-matching extensions are not shown at all
            }
        }

        dirs.sort_by_key(|e| e.name.to_lowercase());
        files.sort_by_key(|e| e.name.to_lowercase());

        self.entries = dirs;
        self.entries.extend(files);
        self.selected = 0;
        self.scroll = 0;
    }

    fn selected_path(&self) -> Option<PathBuf> {
        self.entries.get(self.selected).map(|e| self.current_dir.join(&e.name))
    }

    fn go_up(&mut self) {
        if let Some(parent) = self.current_dir.parent() {
            self.current_dir = parent.to_path_buf();
            self.refresh_entries();
        }
    }

    fn enter_selected(&mut self) {
        if let Some(entry) = self.entries.get(self.selected) {
            if entry.is_dir {
                self.current_dir = self.current_dir.join(&entry.name);
                self.refresh_entries();
            }
        }
    }
}

/// Check if a script file looks compatible: first 32 lines must contain
/// a shebang (`#!/...`) or at least one `#SBATCH` directive.
fn check_script_compatible(path: &std::path::Path) -> bool {
    use std::io::BufRead;
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return false,
    };
    let reader = std::io::BufReader::new(file);
    let mut found_shebang = false;
    let mut found_sbatch = false;
    for (i, line) in reader.lines().enumerate() {
        if i >= 32 { break; }
        let line = match line {
            Ok(l) => l,
            Err(_) => return false, // binary file or encoding error
        };
        let trimmed = line.trim();
        if i == 0 && trimmed.starts_with("#!") {
            found_shebang = true;
        }
        if trimmed.starts_with("#SBATCH") {
            found_sbatch = true;
            break;
        }
    }
    found_shebang || found_sbatch
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

impl ComposerState {
    pub fn new() -> Self {
        let mut fields = HashMap::new();
        fields.insert("mode".into(), "sbatch".into());
        fields.insert("time".into(), "01:00:00".into());
        fields.insert("nodes".into(), "1".into());
        fields.insert("ntasks".into(), "1".into());
        fields.insert("cpus".into(), "1".into());
        fields.insert("memory".into(), "4G".into());
        fields.insert("gpus".into(), String::new());
        fields.insert("name".into(), "my_job".into());
        fields.insert("script".into(), String::new());
        fields.insert("output".into(), "%x-%j.out".into());
        fields.insert("error".into(), "%x-%j.err".into());
        fields.insert("modules".into(), String::new());
        fields.insert("env".into(), String::new());
        fields.insert("init".into(), String::new());
        fields.insert("partition".into(), String::new());
        Self {
            fields,
            partitions: Vec::new(),
            editing: false,
            active_pane: Pane::Form,
            focus: 0,
            mode_is_srun: false,
            template_dialog: None,
            preview_text: String::new(),
            preview_cursor: 0,
            preview_scroll: 0,
            preview_dirty: false,
            field_cursor: 0,
            field_scroll: 0,
            extra_params: Vec::new(),
            help_overlay: false,
            add_param_dialog: None,
            file_browser: None,
            field_editor: None,
        }
    }

    fn visible_fields(&self) -> Vec<Field> {
        Field::all()
            .iter()
            .copied()
            .filter(|f| !self.mode_is_srun || !f.is_sbatch_only())
            .collect()
    }

    fn current_field(&self) -> Field {
        let vis = self.visible_fields();
        vis.get(self.focus).copied().unwrap_or(Field::Mode)
    }

    fn field_key(f: Field) -> &'static str {
        match f {
            Field::Mode => "mode",
            Field::Partition => "partition",
            Field::Time => "time",
            Field::Nodes => "nodes",
            Field::Ntasks => "ntasks",
            Field::Cpus => "cpus",
            Field::Memory => "memory",
            Field::Gpus => "gpus",
            Field::Name => "name",
            Field::Script => "script",
            Field::Output => "output",
            Field::Error => "error",
            Field::Modules => "modules",
            Field::Env => "env",
            Field::Init => "init",
        }
    }

    fn get(&self, f: Field) -> String {
        self.fields
            .get(Self::field_key(f))
            .cloned()
            .unwrap_or_default()
    }

    fn set(&mut self, f: Field, val: String) {
        self.fields.insert(Self::field_key(f).to_string(), val);
    }

    pub fn set_form_state(&mut self, state: &HashMap<String, String>) {
        for (k, v) in state {
            self.fields.insert(k.clone(), v.clone());
        }
        self.mode_is_srun = self.fields.get("mode").is_some_and(|m| m == "srun");
        self.sync_preview_from_form();
    }

    /// Regenerate preview text from form fields
    pub fn sync_preview_from_form(&mut self) {
        self.preview_text = self.generate_preview();
        self.preview_dirty = false;
        // Clamp cursor
        if self.preview_cursor > self.preview_text.len() {
            self.preview_cursor = self.preview_text.len();
        }
    }

    /// Parse preview text back into form fields
    fn sync_form_from_preview(&mut self) {
        let parsed = sbatch_parser::parse_sbatch_text(&self.preview_text);
        for (k, v) in &parsed {
            self.fields.insert(k.clone(), v.clone());
        }
        self.mode_is_srun = self.fields.get("mode").is_some_and(|m| m == "srun");
        self.preview_dirty = false;
    }

    fn build_params(&self) -> HashMap<String, String> {
        let mut params = HashMap::new();
        let val = |key: &str| -> String {
            self.fields.get(key).cloned().unwrap_or_default().trim().to_string()
        };
        if !val("partition").is_empty() {
            params.insert("partition".into(), val("partition"));
        }
        if !val("time").is_empty() {
            params.insert("time".into(), val("time"));
        }
        if !val("nodes").is_empty() {
            params.insert("nodes".into(), val("nodes"));
        }
        if !val("ntasks").is_empty() {
            params.insert("ntasks-per-node".into(), val("ntasks"));
        }
        if !val("cpus").is_empty() {
            params.insert("cpus-per-task".into(), val("cpus"));
        }
        if !val("memory").is_empty() {
            params.insert("mem".into(), val("memory"));
        }
        if !val("gpus").is_empty() {
            params.insert("gres".into(), format!("gpu:{}", val("gpus")));
        }
        if !self.mode_is_srun {
            if !val("name").is_empty() {
                params.insert("job-name".into(), val("name"));
            }
            if !val("output").is_empty() {
                params.insert("output".into(), val("output"));
            }
            if !val("error").is_empty() {
                params.insert("error".into(), val("error"));
            }
        }
        // Include extra params from catalog
        for (key, value) in &self.extra_params {
            if !value.is_empty() || param_catalog::lookup(key).is_some_and(|p| p.is_flag) {
                params.insert(key.clone(), value.clone());
            }
        }
        params
    }

    fn build_wrap_commands(&self) -> String {
        let mut lines: Vec<String> = vec!["#!/bin/bash".to_string()];
        let modules = self.fields.get("modules").cloned().unwrap_or_default();
        for m in modules.lines() {
            let m = m.trim();
            if !m.is_empty() {
                lines.push(format!("module load {m}"));
            }
        }
        let env_str = self.fields.get("env").cloned().unwrap_or_default();
        for e in env_str.lines() {
            let e = e.trim();
            if !e.is_empty() {
                lines.push(format!("export {e}"));
            }
        }
        let init = self.fields.get("init").cloned().unwrap_or_default();
        for c in init.lines() {
            lines.push(c.to_string());
        }
        lines.join("\n")
    }

    fn validate(&self) -> Option<String> {
        if !self.mode_is_srun {
            let name = self.get(Field::Name);
            if let Err(e) = validate_job_name(&name) {
                return Some(e);
            }
            let script = self.get(Field::Script);
            let init = self.get(Field::Init);
            if script.is_empty() && init.is_empty() {
                return Some("Provide a script path or init commands".into());
            }
        }
        let t = self.get(Field::Time);
        if !t.is_empty() && parse_time(&t).is_err() {
            return Some(format!("Invalid time format: {t:?}"));
        }
        let mem = self.get(Field::Memory);
        if !mem.is_empty() && parse_memory(&mem).is_err() {
            return Some(format!("Invalid memory format: {mem:?}"));
        }
        let part = self.get(Field::Partition);
        if part.is_empty() {
            return Some("Select a partition".into());
        }
        None
    }

    /// Argv for the interactive srun session. The preview keeps a literal
    /// "$SHELL" for readability; at launch time the shell is resolved from
    /// the environment (srun does not expand it for us).
    fn build_srun_args(&self, resolve_shell: bool) -> Vec<String> {
        let params = self.build_params();
        let mut sorted: Vec<_> = params.into_iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        let mut parts = vec!["srun".to_string(), "--pty".to_string()];
        for (k, v) in &sorted {
            if v.is_empty() {
                parts.push(format!("--{k}"));
            } else {
                parts.push(format!("--{k}={v}"));
            }
        }
        parts.push(if resolve_shell {
            std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
        } else {
            "$SHELL".to_string()
        });
        parts
    }

    fn generate_preview(&self) -> String {
        if self.mode_is_srun {
            self.build_srun_args(false).join(" \\\n  ")
        } else {
            let mut lines = vec!["#!/bin/bash".to_string()];
            let params = self.build_params();
            let mut sorted: Vec<_> = params.into_iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            for (k, v) in &sorted {
                if v.is_empty() {
                    lines.push(format!("#SBATCH --{k}"));
                } else {
                    lines.push(format!("#SBATCH --{k}={v}"));
                }
            }
            let modules = self.get(Field::Modules);
            let env_str = self.get(Field::Env);
            let init = self.get(Field::Init);
            if !modules.is_empty() || !env_str.is_empty() || !init.is_empty() {
                lines.push(String::new());
            }
            for m in modules.lines() {
                let m = m.trim();
                if !m.is_empty() {
                    lines.push(format!("module load {m}"));
                }
            }
            if !env_str.is_empty() {
                lines.push(String::new());
                for e in env_str.lines() {
                    let e = e.trim();
                    if !e.is_empty() {
                        lines.push(format!("export {e}"));
                    }
                }
            }
            if !init.is_empty() {
                lines.push(String::new());
                for c in init.lines() {
                    lines.push(c.to_string());
                }
            }
            let script = self.get(Field::Script);
            if !script.is_empty() {
                lines.push(String::new());
                lines.push(script);
            }
            lines.join("\n")
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent, _slurm: &dyn SlurmController) -> Action {
        // Help overlay takes priority
        if self.help_overlay {
            match key.code {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') => {
                    self.help_overlay = false;
                }
                _ => {}
            }
            return Action::None;
        }

        // File browser dialog
        if let Some(ref mut fb) = self.file_browser {
            match key.code {
                KeyCode::Esc => {
                    self.file_browser = None;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if fb.selected + 1 < fb.entries.len() {
                        fb.selected += 1;
                        // Keep selection visible
                        let visible_h = 20_usize; // approximate
                        if fb.selected >= fb.scroll + visible_h {
                            fb.scroll = fb.selected + 1 - visible_h;
                        }
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    fb.selected = fb.selected.saturating_sub(1);
                    if fb.selected < fb.scroll {
                        fb.scroll = fb.selected;
                    }
                }
                KeyCode::Left | KeyCode::Backspace => {
                    fb.go_up();
                }
                KeyCode::Right => {
                    // Enter directory on Right arrow
                    if fb.entries.get(fb.selected).is_some_and(|e| e.is_dir) {
                        fb.enter_selected();
                    }
                }
                KeyCode::Enter => {
                    if let Some(entry) = fb.entries.get(fb.selected) {
                        if entry.is_dir {
                            fb.enter_selected();
                        } else if entry.is_compatible {
                            let path = fb.selected_path().unwrap();
                            let path_str = path.to_string_lossy().to_string();
                            self.file_browser = None;
                            match sbatch_parser::parse_sbatch_file(&path_str) {
                                Ok(state) => {
                                    self.set_form_state(&state);
                                    // Auto-switch to preview pane in edit mode
                                    self.active_pane = Pane::Preview;
                                    self.editing = true;
                                    self.preview_cursor = self.preview_text.len();
                                    self.preview_scroll = 0;
                                    return Action::Status(format!("Loaded {path_str}"));
                                }
                                Err(e) => {
                                    return Action::Status(format!("! {e}"));
                                }
                            }
                        }
                        // incompatible files: do nothing on Enter
                    }
                }
                KeyCode::Char('~') => {
                    // Go to home directory
                    if let Some(home) = dirs_home() {
                        fb.current_dir = home;
                        fb.refresh_entries();
                    }
                }
                KeyCode::Char('.') => {
                    // Toggle showing hidden files: re-scan including dot-files
                    // For now, just refresh
                    fb.refresh_entries();
                }
                _ => {}
            }
            return Action::None;
        }

        // Multiline field editor (right pane)
        if self.field_editor.is_some() {
            return self.handle_key_field_editor(key);
        }

        // Add parameter dialog
        if let Some(ref mut dialog) = self.add_param_dialog {
            let search = dialog.search.clone();
            let filtered = self.filtered_catalog_entries(&search);
            let dialog = self.add_param_dialog.as_mut().unwrap();
            match key.code {
                KeyCode::Esc => {
                    self.add_param_dialog = None;
                }
                KeyCode::Enter => {
                    if let Some(&idx) = filtered.get(dialog.selected) {
                        let entry = &param_catalog::ALL_PARAMS[idx];
                        let key = entry.key.to_string();
                        self.extra_params.push((key, String::new()));
                        self.add_param_dialog = None;
                        self.sync_preview_from_form();
                        return Action::Status(format!("Added --{}", entry.key));
                    }
                }
                KeyCode::Down | KeyCode::Char('j') if dialog.search.is_empty() => {
                    if dialog.selected + 1 < filtered.len() {
                        dialog.selected += 1;
                    }
                }
                KeyCode::Up | KeyCode::Char('k') if dialog.search.is_empty() => {
                    dialog.selected = dialog.selected.saturating_sub(1);
                }
                KeyCode::Down => {
                    if dialog.selected + 1 < filtered.len() {
                        dialog.selected += 1;
                    }
                }
                KeyCode::Up => {
                    dialog.selected = dialog.selected.saturating_sub(1);
                }
                KeyCode::Backspace => {
                    dialog.search.pop();
                    dialog.selected = 0;
                }
                KeyCode::Char(c) => {
                    dialog.search.push(c);
                    dialog.selected = 0;
                }
                _ => {}
            }
            return Action::None;
        }

        // Template dialog takes priority
        if let Some(ref mut dialog) = self.template_dialog {
            match dialog {
                TemplateDialog::Save { ref mut name } => {
                    match key.code {
                        KeyCode::Esc => {
                            self.template_dialog = None;
                        }
                        KeyCode::Enter => {
                            let n = name.clone();
                            if !n.is_empty() {
                                let _ = templates::save_template(&n, &self.fields);
                                self.template_dialog = None;
                                return Action::Status(format!("Template '{n}' saved"));
                            }
                        }
                        KeyCode::Backspace => { name.pop(); }
                        KeyCode::Char(c) => { name.push(c); }
                        _ => {}
                    }
                    return Action::None;
                }
                TemplateDialog::Load { ref names, ref mut selected } => {
                    match key.code {
                        KeyCode::Esc => {
                            self.template_dialog = None;
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if *selected + 1 < names.len() {
                                *selected += 1;
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            *selected = selected.saturating_sub(1);
                        }
                        KeyCode::Enter => {
                            if let Some(name) = names.get(*selected).cloned() {
                                if let Some(data) = templates::load_template(&name) {
                                    self.set_form_state(&data);
                                    self.template_dialog = None;
                                    return Action::Status(format!("Template '{name}' loaded"));
                                }
                            }
                        }
                        KeyCode::Char('d') => {
                            if let Some(name) = names.get(*selected).cloned() {
                                templates::delete_template(&name);
                                let new_names = templates::list_templates();
                                if new_names.is_empty() {
                                    self.template_dialog = None;
                                } else {
                                    let sel = (*selected).min(new_names.len().saturating_sub(1));
                                    self.template_dialog = Some(TemplateDialog::Load {
                                        names: new_names,
                                        selected: sel,
                                    });
                                }
                                return Action::Status(format!("Template '{name}' deleted"));
                            }
                        }
                        _ => {}
                    }
                    return Action::None;
                }
            }
        }

        // Ctrl+S: submit
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
            // If preview was manually edited, sync back to form first
            if self.preview_dirty {
                self.sync_form_from_preview();
            }
            if let Some(err) = self.validate() {
                return Action::Status(format!("! {err}"));
            }
            if self.mode_is_srun {
                // Interactive mode launches srun with the real terminal —
                // submitting the wrap script via sbatch would silently turn
                // the advertised interactive session into a batch job.
                return Action::RunInteractive(self.build_srun_args(true));
            }
            let params = self.build_params();
            let script = self.get(Field::Script);
            if script.is_empty() {
                // sbatch mode without script path: submit full generated script as temp file
                let body = self.generate_preview();
                return Action::Submit(params, String::new(), body);
            }
            let wrap = self.build_wrap_commands();
            return Action::Submit(params, script, wrap);
        }

        // Ctrl+T: save template
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('t') {
            if self.preview_dirty {
                self.sync_form_from_preview();
            }
            self.template_dialog = Some(TemplateDialog::Save {
                name: String::new(),
            });
            return Action::None;
        }

        // Ctrl+L: load template
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('l') {
            let names = templates::list_templates();
            if names.is_empty() {
                return Action::Status("No templates saved".into());
            }
            self.template_dialog = Some(TemplateDialog::Load {
                names,
                selected: 0,
            });
            return Action::None;
        }

        // Ctrl+O: open file browser
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('o') {
            self.file_browser = Some(FileBrowserDialog::new());
            return Action::None;
        }

        // Ctrl+Y: copy preview to clipboard (OSC 52)
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('y') {
            use std::io::Write;
            let encoded = base64_encode(self.preview_text.as_bytes());
            let _ = write!(std::io::stdout(), "\x1b]52;c;{encoded}\x07");
            let _ = std::io::stdout().flush();
            return Action::Status("Preview copied to clipboard".into());
        }

        match self.active_pane {
            Pane::Form => self.handle_key_form(key),
            Pane::Preview => self.handle_key_preview(key),
        }
    }

    fn handle_key_form(&mut self, key: KeyEvent) -> Action {
        let vis = self.visible_fields();
        let total_fields = vis.len() + self.extra_params.len();
        if total_fields == 0 {
            return Action::None;
        }
        let field = self.current_field();
        let in_extra = self.focus >= vis.len();

        // When NOT editing: navigation only
        if !self.editing {
            match key.code {
                KeyCode::Tab => {
                    // Switch to preview pane
                    if self.active_pane == Pane::Form {
                        // Sync preview from form before switching
                        if !self.preview_dirty {
                            self.sync_preview_from_form();
                        }
                        self.active_pane = Pane::Preview;
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.focus = (self.focus + 1) % total_fields;
                    self.field_scroll = 0;
                }
                KeyCode::BackTab | KeyCode::Up | KeyCode::Char('k') => {
                    self.focus = if self.focus == 0 { total_fields.saturating_sub(1) } else { self.focus - 1 };
                    self.field_scroll = 0;
                }
                KeyCode::Enter | KeyCode::Char(' ') => {
                    if in_extra {
                        let entry_key = self.extra_params[self.focus - vis.len()].0.clone();
                        if !param_catalog::lookup(&entry_key).is_some_and(|p| p.is_flag) {
                            self.editing = true;
                            self.field_cursor = self.extra_params[self.focus - vis.len()].1.len();
                            self.field_scroll = 0;
                        }
                    } else {
                        match field {
                            Field::Mode => {
                                self.mode_is_srun = !self.mode_is_srun;
                                self.fields.insert(
                                    "mode".into(),
                                    if self.mode_is_srun { "srun" } else { "sbatch" }.into(),
                                );
                                self.focus = self.focus.min(self.visible_fields().len().saturating_sub(1));
                                self.sync_preview_from_form();
                            }
                            Field::Partition => {
                                if !self.partitions.is_empty() {
                                    let cur = self.get(Field::Partition);
                                    let idx = self.partitions.iter().position(|p| p == &cur);
                                    let new_idx = match idx {
                                        Some(i) if i + 1 < self.partitions.len() => i + 1,
                                        _ => 0,
                                    };
                                    self.set(Field::Partition, self.partitions[new_idx].clone());
                                    self.sync_preview_from_form();
                                }
                            }
                            // Multiline fields open in the right-pane editor
                            Field::Modules | Field::Env | Field::Init => {
                                let text = self.get(field);
                                let cursor = text.len();
                                self.field_editor = Some(FieldEditor {
                                    field,
                                    text,
                                    cursor,
                                    scroll: 0,
                                });
                                self.editing = true;
                            }
                            _ => {
                                self.editing = true;
                                self.field_cursor = self.get(field).len();
                                self.field_scroll = 0;
                            }
                        }
                    }
                }
                KeyCode::Left => {
                    if field == Field::Partition && !self.partitions.is_empty() {
                        let cur = self.get(Field::Partition);
                        let idx = self.partitions.iter().position(|p| p == &cur);
                        let new_idx = match idx {
                            Some(0) | None => self.partitions.len().saturating_sub(1),
                            Some(i) => i - 1,
                        };
                        self.set(Field::Partition, self.partitions[new_idx].clone());
                        self.sync_preview_from_form();
                    } else if field == Field::Mode {
                        self.mode_is_srun = !self.mode_is_srun;
                        self.fields.insert(
                            "mode".into(),
                            if self.mode_is_srun { "srun" } else { "sbatch" }.into(),
                        );
                        self.focus = self.focus.min(self.visible_fields().len().saturating_sub(1));
                        self.sync_preview_from_form();
                    }
                }
                KeyCode::Right => {
                    if field == Field::Partition && !self.partitions.is_empty() {
                        let cur = self.get(Field::Partition);
                        let idx = self.partitions.iter().position(|p| p == &cur);
                        let new_idx = match idx {
                            Some(i) if i + 1 < self.partitions.len() => i + 1,
                            _ => 0,
                        };
                        self.set(Field::Partition, self.partitions[new_idx].clone());
                        self.sync_preview_from_form();
                    } else if field == Field::Mode {
                        self.mode_is_srun = !self.mode_is_srun;
                        self.fields.insert(
                            "mode".into(),
                            if self.mode_is_srun { "srun" } else { "sbatch" }.into(),
                        );
                        self.focus = self.focus.min(self.visible_fields().len().saturating_sub(1));
                        self.sync_preview_from_form();
                    }
                }
                KeyCode::Char('?') => {
                    self.help_overlay = true;
                }
                KeyCode::Char('a') => {
                    self.add_param_dialog = Some(AddParamDialog {
                        search: String::new(),
                        selected: 0,
                    });
                }
                KeyCode::Char('d') => {
                    // Delete focused extra param (only if focus is on an extra param)
                    let extra_idx = self.focus as isize - vis.len() as isize;
                    if extra_idx >= 0 && (extra_idx as usize) < self.extra_params.len() {
                        self.extra_params.remove(extra_idx as usize);
                        if self.focus > 0 {
                            self.focus -= 1;
                        }
                        self.sync_preview_from_form();
                    }
                }
                _ => {}
            }
            return Action::None;
        }

        // When EDITING: text input on current field with cursor support
        match key.code {
            KeyCode::Esc => {
                self.editing = false;
                self.sync_preview_from_form();
            }
            KeyCode::Tab => {
                self.editing = false;
                self.focus = (self.focus + 1) % total_fields;
                self.field_scroll = 0;
                self.sync_preview_from_form();
            }
            KeyCode::BackTab => {
                self.editing = false;
                self.focus = if self.focus == 0 { total_fields.saturating_sub(1) } else { self.focus - 1 };
                self.field_scroll = 0;
                self.sync_preview_from_form();
            }
            _ => {
                // Get mutable reference to the value being edited
                let (val, is_multiline) = if in_extra {
                    let extra_idx = self.focus - vis.len();
                    (self.extra_params[extra_idx].1.clone(), false)
                } else {
                    let key_name = Self::field_key(field);
                    let v = self.fields.get(key_name).cloned().unwrap_or_default();
                    let multi = matches!(field, Field::Modules | Field::Env | Field::Init);
                    (v, multi)
                };

                let mut new_val = val;
                // Clamp cursor
                self.field_cursor = self.field_cursor.min(new_val.len());

                match key.code {
                    KeyCode::Char(c) if c != '\r' => {
                        new_val.insert(self.field_cursor, c);
                        self.field_cursor += c.len_utf8();
                    }
                    KeyCode::Backspace => {
                        if self.field_cursor > 0 {
                            let prev = new_val[..self.field_cursor]
                                .char_indices()
                                .last()
                                .map(|(i, _)| i)
                                .unwrap_or(0);
                            new_val.remove(prev);
                            self.field_cursor = prev;
                        }
                    }
                    KeyCode::Delete => {
                        if self.field_cursor < new_val.len() {
                            new_val.remove(self.field_cursor);
                        }
                    }
                    KeyCode::Enter => {
                        if is_multiline {
                            new_val.insert(self.field_cursor, '\n');
                            self.field_cursor += 1;
                        } else {
                            self.editing = false;
                            self.focus = (self.focus + 1) % total_fields;
                            self.field_scroll = 0;
                            self.sync_preview_from_form();
                            return Action::None;
                        }
                    }
                    KeyCode::Left => {
                        if self.field_cursor > 0 {
                            self.field_cursor = new_val[..self.field_cursor]
                                .char_indices()
                                .last()
                                .map(|(i, _)| i)
                                .unwrap_or(0);
                        }
                    }
                    KeyCode::Right => {
                        if self.field_cursor < new_val.len() {
                            self.field_cursor += new_val[self.field_cursor..]
                                .chars()
                                .next()
                                .map(|c| c.len_utf8())
                                .unwrap_or(0);
                        }
                    }
                    KeyCode::Home => {
                        // Move to start of current line
                        let before = &new_val[..self.field_cursor];
                        self.field_cursor = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
                    }
                    KeyCode::End => {
                        // Move to end of current line
                        let after = &new_val[self.field_cursor..];
                        self.field_cursor += after.find('\n').unwrap_or(after.len());
                    }
                    KeyCode::Up if is_multiline => {
                        let before = &new_val[..self.field_cursor];
                        let cur_line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
                        let col = self.field_cursor - cur_line_start;
                        if cur_line_start > 0 {
                            let prev_line_start = new_val[..cur_line_start - 1]
                                .rfind('\n')
                                .map(|i| i + 1)
                                .unwrap_or(0);
                            let prev_line_len = cur_line_start - 1 - prev_line_start;
                            self.field_cursor = prev_line_start + col.min(prev_line_len);
                        }
                    }
                    KeyCode::Down if is_multiline => {
                        let before = &new_val[..self.field_cursor];
                        let cur_line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
                        let col = self.field_cursor - cur_line_start;
                        let after = &new_val[self.field_cursor..];
                        if let Some(nl) = after.find('\n') {
                            let next_line_start = self.field_cursor + nl + 1;
                            let next_after = &new_val[next_line_start..];
                            let next_line_len = next_after.find('\n').unwrap_or(next_after.len());
                            self.field_cursor = next_line_start + col.min(next_line_len);
                        }
                    }
                    _ => {}
                }

                // Adjust scroll for multiline fields to keep cursor visible
                if is_multiline {
                    const MAX_VIS: usize = 8;
                    let cursor_line = new_val[..self.field_cursor.min(new_val.len())].matches('\n').count();
                    if cursor_line < self.field_scroll {
                        self.field_scroll = cursor_line;
                    } else if cursor_line >= self.field_scroll + MAX_VIS {
                        self.field_scroll = cursor_line + 1 - MAX_VIS;
                    }
                }

                // Write back
                if in_extra {
                    let extra_idx = self.focus - vis.len();
                    self.extra_params[extra_idx].1 = new_val;
                } else {
                    let key_name = Self::field_key(field);
                    self.fields.insert(key_name.to_string(), new_val);
                }
            }
        }
        Action::None
    }

    /// Handle pasted text (from bracketed paste mode).
    /// Normalizes CRLF → LF and strips stray CR before inserting.
    pub fn handle_paste(&mut self, text: &str) {
        let clean = text.replace("\r\n", "\n").replace('\r', "\n");
        if self.active_pane == Pane::Preview && self.editing {
            for c in clean.chars() {
                if self.preview_cursor <= self.preview_text.len() {
                    self.preview_text.insert(self.preview_cursor, c);
                    self.preview_cursor += c.len_utf8();
                }
            }
            self.preview_dirty = true;
        } else if self.active_pane == Pane::Form && self.editing {
            let vis = self.visible_fields();
            if self.focus < vis.len() {
                let field = vis[self.focus];
                let key_name = Self::field_key(field);
                let is_multiline = matches!(field, Field::Modules | Field::Env | Field::Init);
                let mut val = self.fields.get(key_name).cloned().unwrap_or_default();
                self.field_cursor = self.field_cursor.min(val.len());
                for c in clean.chars() {
                    if c == '\n' && !is_multiline {
                        continue;
                    }
                    val.insert(self.field_cursor, c);
                    self.field_cursor += c.len_utf8();
                }
                self.fields.insert(key_name.to_string(), val.clone());
                // Adjust scroll for multiline fields
                if is_multiline {
                    const MAX_VIS: usize = 8;
                    let cursor_line = val[..self.field_cursor.min(val.len())].matches('\n').count();
                    if cursor_line >= self.field_scroll + MAX_VIS {
                        self.field_scroll = cursor_line + 1 - MAX_VIS;
                    }
                }
            }
        }
    }

    fn handle_key_preview(&mut self, key: KeyEvent) -> Action {
        if !self.editing {
            // Navigation mode in preview pane
            match key.code {
                KeyCode::Tab | KeyCode::BackTab => {
                    // Switch back to form pane
                    if self.preview_dirty {
                        self.sync_form_from_preview();
                    }
                    self.active_pane = Pane::Form;
                }
                KeyCode::Enter => {
                    // Start editing the preview text
                    self.editing = true;
                    self.preview_cursor = self.preview_text.len();
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let line_count = self.preview_text.lines().count().max(1);
                    if self.preview_scroll + 1 < line_count {
                        self.preview_scroll += 1;
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.preview_scroll = self.preview_scroll.saturating_sub(1);
                }
                _ => {}
            }
            return Action::None;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Editing mode in preview - direct text editing
        match key.code {
            KeyCode::Esc => {
                self.editing = false;
                if self.preview_dirty {
                    self.sync_form_from_preview();
                }
            }

            // Ctrl+K: delete from cursor to end of line
            KeyCode::Char('k') if ctrl => {
                let after = &self.preview_text[self.preview_cursor..];
                let eol = after.find('\n').unwrap_or(after.len());
                if eol == 0 && self.preview_cursor < self.preview_text.len() {
                    // Cursor at newline: delete the newline itself
                    self.preview_text.remove(self.preview_cursor);
                } else {
                    self.preview_text.replace_range(self.preview_cursor..self.preview_cursor + eol, "");
                }
                self.preview_dirty = true;
            }

            // Ctrl+U: delete from cursor to start of line
            KeyCode::Char('u') if ctrl => {
                let before = &self.preview_text[..self.preview_cursor];
                let sol = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
                self.preview_text.replace_range(sol..self.preview_cursor, "");
                self.preview_cursor = sol;
                self.preview_dirty = true;
            }

            // Ctrl+D: delete entire current line
            KeyCode::Char('d') if ctrl => {
                let before = &self.preview_text[..self.preview_cursor];
                let sol = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
                let after = &self.preview_text[self.preview_cursor..];
                let eol = after.find('\n').map(|i| self.preview_cursor + i + 1)
                    .unwrap_or(self.preview_text.len());
                // If deleting last line and there's a preceding newline, remove it too
                let start = if sol > 0 && eol == self.preview_text.len() { sol - 1 } else { sol };
                self.preview_text.replace_range(start..eol, "");
                self.preview_cursor = start.min(self.preview_text.len());
                self.preview_dirty = true;
            }

            // Ctrl+Backspace / Ctrl+W: delete previous word
            KeyCode::Backspace if ctrl => {
                if self.preview_cursor > 0 {
                    let before = &self.preview_text[..self.preview_cursor];
                    let trimmed = before.trim_end();
                    let word_start = trimmed.rfind(|c: char| c.is_whitespace() || c == '/' || c == '-')
                        .map(|i| i + 1)
                        .unwrap_or(0);
                    self.preview_text.replace_range(word_start..self.preview_cursor, "");
                    self.preview_cursor = word_start;
                    self.preview_dirty = true;
                }
            }
            KeyCode::Char('w') if ctrl => {
                // Same as Ctrl+Backspace
                if self.preview_cursor > 0 {
                    let before = &self.preview_text[..self.preview_cursor];
                    let trimmed = before.trim_end();
                    let word_start = trimmed.rfind(|c: char| c.is_whitespace() || c == '/' || c == '-')
                        .map(|i| i + 1)
                        .unwrap_or(0);
                    self.preview_text.replace_range(word_start..self.preview_cursor, "");
                    self.preview_cursor = word_start;
                    self.preview_dirty = true;
                }
            }

            // Ctrl+Left: move to previous word boundary
            KeyCode::Left if ctrl => {
                if self.preview_cursor > 0 {
                    let before = &self.preview_text[..self.preview_cursor];
                    let trimmed_len = before.trim_end().len();
                    let search_in = &self.preview_text[..trimmed_len];
                    self.preview_cursor = search_in
                        .rfind(|c: char| c.is_whitespace() || c == '/' || c == '-' || c == '=')
                        .map(|i| i + 1)
                        .unwrap_or(0);
                }
            }

            // Ctrl+Right: move to next word boundary
            KeyCode::Right if ctrl => {
                if self.preview_cursor < self.preview_text.len() {
                    let after = &self.preview_text[self.preview_cursor..];
                    // Skip current word chars, then skip whitespace
                    let skip_word = after
                        .find(|c: char| c.is_whitespace() || c == '/' || c == '-' || c == '=')
                        .unwrap_or(after.len());
                    let rest = &after[skip_word..];
                    let skip_space = rest
                        .find(|c: char| !c.is_whitespace())
                        .unwrap_or(rest.len());
                    self.preview_cursor += skip_word + skip_space;
                }
            }

            // Ctrl+A: move to start of line (like shell)
            KeyCode::Char('a') if ctrl => {
                let before = &self.preview_text[..self.preview_cursor];
                self.preview_cursor = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
            }

            // Ctrl+E: move to end of line (like shell)
            KeyCode::Char('e') if ctrl => {
                let after = &self.preview_text[self.preview_cursor..];
                self.preview_cursor += after.find('\n').unwrap_or(after.len());
            }

            KeyCode::Tab => {
                // Insert 4 spaces for indentation
                let indent = "    ";
                self.preview_text.insert_str(self.preview_cursor, indent);
                self.preview_cursor += indent.len();
                self.preview_dirty = true;
            }

            KeyCode::BackTab => {
                // Remove up to 4 leading spaces on current line
                let before = &self.preview_text[..self.preview_cursor];
                let sol = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
                let line_start = &self.preview_text[sol..];
                let spaces = line_start.chars().take(4).take_while(|c| *c == ' ').count();
                if spaces > 0 {
                    self.preview_text.replace_range(sol..sol + spaces, "");
                    self.preview_cursor = self.preview_cursor.saturating_sub(spaces);
                    self.preview_dirty = true;
                }
            }

            KeyCode::Char(c) if c != '\r' && !ctrl => {
                if self.preview_cursor <= self.preview_text.len() {
                    self.preview_text.insert(self.preview_cursor, c);
                    self.preview_cursor += c.len_utf8();
                    self.preview_dirty = true;
                }
            }
            KeyCode::Backspace => {
                if self.preview_cursor > 0 {
                    let prev = self.preview_text[..self.preview_cursor]
                        .char_indices()
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                    self.preview_text.remove(prev);
                    self.preview_cursor = prev;
                    self.preview_dirty = true;
                }
            }
            KeyCode::Delete => {
                if self.preview_cursor < self.preview_text.len() {
                    self.preview_text.remove(self.preview_cursor);
                    self.preview_dirty = true;
                }
            }
            KeyCode::Enter => {
                // Auto-indent: copy leading whitespace from current line
                let before = &self.preview_text[..self.preview_cursor];
                let sol = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
                let current_line = &self.preview_text[sol..];
                let indent: String = current_line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
                let insert = format!("\n{indent}");
                self.preview_text.insert_str(self.preview_cursor, &insert);
                self.preview_cursor += insert.len();
                self.preview_dirty = true;
            }
            KeyCode::Left => {
                if self.preview_cursor > 0 {
                    self.preview_cursor = self.preview_text[..self.preview_cursor]
                        .char_indices()
                        .last()
                        .map(|(i, _)| i)
                        .unwrap_or(0);
                }
            }
            KeyCode::Right => {
                if self.preview_cursor < self.preview_text.len() {
                    self.preview_cursor += self.preview_text[self.preview_cursor..]
                        .chars()
                        .next()
                        .map(|c| c.len_utf8())
                        .unwrap_or(0);
                }
            }
            KeyCode::Home => {
                let before = &self.preview_text[..self.preview_cursor];
                self.preview_cursor = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
            }
            KeyCode::End => {
                let after = &self.preview_text[self.preview_cursor..];
                self.preview_cursor += after.find('\n').unwrap_or(after.len());
            }
            KeyCode::Up => {
                let before = &self.preview_text[..self.preview_cursor];
                let cur_line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
                let col = self.preview_cursor - cur_line_start;
                if cur_line_start > 0 {
                    let prev_line_start = self.preview_text[..cur_line_start - 1]
                        .rfind('\n')
                        .map(|i| i + 1)
                        .unwrap_or(0);
                    let prev_line_len = cur_line_start - 1 - prev_line_start;
                    self.preview_cursor = prev_line_start + col.min(prev_line_len);
                }
            }
            KeyCode::Down => {
                let before = &self.preview_text[..self.preview_cursor];
                let cur_line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
                let col = self.preview_cursor - cur_line_start;
                let after = &self.preview_text[self.preview_cursor..];
                if let Some(nl) = after.find('\n') {
                    let next_line_start = self.preview_cursor + nl + 1;
                    let next_after = &self.preview_text[next_line_start..];
                    let next_line_len = next_after.find('\n').unwrap_or(next_after.len());
                    self.preview_cursor = next_line_start + col.min(next_line_len);
                }
            }
            _ => {}
        }
        Action::None
    }

    /// Handle keys when the multiline field editor is active.
    /// Reuses the same editing logic as the preview editor.
    fn handle_key_field_editor(&mut self, key: KeyEvent) -> Action {
        let fe = self.field_editor.as_mut().unwrap();
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        match key.code {
            KeyCode::Esc => {
                // Save back to field and close editor
                let field = fe.field;
                let text = fe.text.clone();
                self.set(field, text);
                self.field_editor = None;
                self.editing = false;
                self.sync_preview_from_form();
            }
            KeyCode::Char('k') if ctrl => {
                let after = &fe.text[fe.cursor..];
                let eol = after.find('\n').unwrap_or(after.len());
                if eol == 0 && fe.cursor < fe.text.len() {
                    fe.text.remove(fe.cursor);
                } else {
                    fe.text.replace_range(fe.cursor..fe.cursor + eol, "");
                }
            }
            KeyCode::Char('u') if ctrl => {
                let before = &fe.text[..fe.cursor];
                let sol = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
                fe.text.replace_range(sol..fe.cursor, "");
                fe.cursor = sol;
            }
            KeyCode::Char('d') if ctrl => {
                let before = &fe.text[..fe.cursor];
                let sol = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
                let after = &fe.text[fe.cursor..];
                let eol = after.find('\n').map(|i| fe.cursor + i + 1).unwrap_or(fe.text.len());
                let start = if sol > 0 && eol == fe.text.len() { sol - 1 } else { sol };
                fe.text.replace_range(start..eol, "");
                fe.cursor = start.min(fe.text.len());
            }
            KeyCode::Backspace if ctrl => {
                if fe.cursor > 0 {
                    let before = &fe.text[..fe.cursor];
                    let trimmed = before.trim_end();
                    let word_start = trimmed.rfind(|c: char| c.is_whitespace() || c == '/' || c == '-')
                        .map(|i| i + 1).unwrap_or(0);
                    fe.text.replace_range(word_start..fe.cursor, "");
                    fe.cursor = word_start;
                }
            }
            KeyCode::Char('w') if ctrl => {
                if fe.cursor > 0 {
                    let before = &fe.text[..fe.cursor];
                    let trimmed = before.trim_end();
                    let word_start = trimmed.rfind(|c: char| c.is_whitespace() || c == '/' || c == '-')
                        .map(|i| i + 1).unwrap_or(0);
                    fe.text.replace_range(word_start..fe.cursor, "");
                    fe.cursor = word_start;
                }
            }
            KeyCode::Left if ctrl => {
                if fe.cursor > 0 {
                    let before = &fe.text[..fe.cursor];
                    let trimmed_len = before.trim_end().len();
                    let search_in = &fe.text[..trimmed_len];
                    fe.cursor = search_in
                        .rfind(|c: char| c.is_whitespace() || c == '/' || c == '-' || c == '=')
                        .map(|i| i + 1).unwrap_or(0);
                }
            }
            KeyCode::Right if ctrl => {
                if fe.cursor < fe.text.len() {
                    let after = &fe.text[fe.cursor..];
                    let skip_word = after.find(|c: char| c.is_whitespace() || c == '/' || c == '-' || c == '=')
                        .unwrap_or(after.len());
                    let rest = &after[skip_word..];
                    let skip_space = rest.find(|c: char| !c.is_whitespace()).unwrap_or(rest.len());
                    fe.cursor += skip_word + skip_space;
                }
            }
            KeyCode::Char('a') if ctrl => {
                let before = &fe.text[..fe.cursor];
                fe.cursor = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
            }
            KeyCode::Char('e') if ctrl => {
                let after = &fe.text[fe.cursor..];
                fe.cursor += after.find('\n').unwrap_or(after.len());
            }
            KeyCode::Tab => {
                fe.text.insert_str(fe.cursor, "    ");
                fe.cursor += 4;
            }
            KeyCode::BackTab => {
                let before = &fe.text[..fe.cursor];
                let sol = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
                let line_start = &fe.text[sol..];
                let spaces = line_start.chars().take(4).take_while(|c| *c == ' ').count();
                if spaces > 0 {
                    fe.text.replace_range(sol..sol + spaces, "");
                    fe.cursor = fe.cursor.saturating_sub(spaces);
                }
            }
            KeyCode::Char(c) if c != '\r' && !ctrl => {
                if fe.cursor <= fe.text.len() {
                    fe.text.insert(fe.cursor, c);
                    fe.cursor += c.len_utf8();
                }
            }
            KeyCode::Backspace => {
                if fe.cursor > 0 {
                    let prev = fe.text[..fe.cursor].char_indices().last().map(|(i, _)| i).unwrap_or(0);
                    fe.text.remove(prev);
                    fe.cursor = prev;
                }
            }
            KeyCode::Delete => {
                if fe.cursor < fe.text.len() {
                    fe.text.remove(fe.cursor);
                }
            }
            KeyCode::Enter => {
                let before = &fe.text[..fe.cursor];
                let sol = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
                let current_line = &fe.text[sol..];
                let indent: String = current_line.chars().take_while(|c| *c == ' ' || *c == '\t').collect();
                let insert = format!("\n{indent}");
                fe.text.insert_str(fe.cursor, &insert);
                fe.cursor += insert.len();
            }
            KeyCode::Left => {
                if fe.cursor > 0 {
                    fe.cursor = fe.text[..fe.cursor].char_indices().last().map(|(i, _)| i).unwrap_or(0);
                }
            }
            KeyCode::Right => {
                if fe.cursor < fe.text.len() {
                    fe.cursor += fe.text[fe.cursor..].chars().next().map(|c| c.len_utf8()).unwrap_or(0);
                }
            }
            KeyCode::Home => {
                let before = &fe.text[..fe.cursor];
                fe.cursor = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
            }
            KeyCode::End => {
                let after = &fe.text[fe.cursor..];
                fe.cursor += after.find('\n').unwrap_or(after.len());
            }
            KeyCode::Up => {
                let before = &fe.text[..fe.cursor];
                let cur_line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
                let col = fe.cursor - cur_line_start;
                if cur_line_start > 0 {
                    let prev_line_start = fe.text[..cur_line_start - 1].rfind('\n').map(|i| i + 1).unwrap_or(0);
                    let prev_line_len = cur_line_start - 1 - prev_line_start;
                    fe.cursor = prev_line_start + col.min(prev_line_len);
                }
            }
            KeyCode::Down => {
                let before = &fe.text[..fe.cursor];
                let cur_line_start = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
                let col = fe.cursor - cur_line_start;
                let after = &fe.text[fe.cursor..];
                if let Some(nl) = after.find('\n') {
                    let next_line_start = fe.cursor + nl + 1;
                    let next_after = &fe.text[next_line_start..];
                    let next_line_len = next_after.find('\n').unwrap_or(next_after.len());
                    fe.cursor = next_line_start + col.min(next_line_len);
                }
            }
            _ => {}
        }
        Action::None
    }

    pub fn draw(&self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);

        self.draw_form(f, chunks[0]);

        // Right pane: field editor takes priority over preview
        if let Some(ref fe) = self.field_editor {
            self.draw_field_editor(f, chunks[1], fe);
        } else {
            self.draw_preview(f, chunks[1]);
        }

        // Template dialog overlay
        if let Some(ref dialog) = self.template_dialog {
            match dialog {
                TemplateDialog::Save { name } => {
                    let area = centered_rect(40, 5, f.area());
                    let block = Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .title(" Save Template ")
                        .border_style(Style::default().fg(theme::ACCENT))
                        .style(Style::default().bg(theme::SURFACE));
                    let inner = block.inner(area);
                    f.render_widget(Clear, area);
                    f.render_widget(block, area);
                    f.render_widget(
                        Paragraph::new(vec![
                            Line::from(vec![
                                Span::styled("Name: ", Style::default().fg(theme::DIM)),
                                Span::styled(format!("{name}▏"), Style::default().fg(theme::TEXT)),
                            ]),
                            Line::from(""),
                            Line::from(Span::styled(
                                "Enter to save, Esc to cancel",
                                Style::default().fg(theme::MUTED),
                            )),
                        ]),
                        inner,
                    );
                }
                TemplateDialog::Load { names, selected } => {
                    let h = (names.len() as u16 + 4).min(20);
                    let area = centered_rect(40, h, f.area());
                    let block = Block::default()
                        .borders(Borders::ALL)
                        .border_type(BorderType::Rounded)
                        .title(" Load Template ")
                        .title_bottom(" d=delete ")
                        .border_style(Style::default().fg(theme::ACCENT))
                        .style(Style::default().bg(theme::SURFACE));
                    let inner = block.inner(area);
                    f.render_widget(Clear, area);
                    f.render_widget(block, area);
                    let items: Vec<ListItem> = names
                        .iter()
                        .enumerate()
                        .map(|(i, n)| {
                            let style = if i == *selected {
                                Style::default().bg(theme::HIGHLIGHT).fg(theme::TEXT)
                            } else {
                                Style::default().fg(theme::DIM)
                            };
                            ListItem::new(format!("  {n}")).style(style)
                        })
                        .collect();
                    f.render_widget(List::new(items), inner);
                }
            }
        }

        // Help overlay
        if self.help_overlay {
            self.draw_help_overlay(f, f.area());
        }

        // Add parameter dialog
        if let Some(ref dialog) = self.add_param_dialog {
            self.draw_add_param_dialog(f, f.area(), dialog);
        }

        // File browser dialog
        if let Some(ref fb) = self.file_browser {
            self.draw_file_browser(f, f.area(), fb);
        }
    }

    fn draw_form(&self, f: &mut Frame, area: Rect) {
        let vis = self.visible_fields();
        let is_active = self.active_pane == Pane::Form;
        let border_color = if is_active && self.editing {
            theme::GREEN
        } else if is_active {
            theme::ACCENT
        } else {
            theme::BORDER
        };
        let title = if is_active && self.editing {
            " Form [editing] "
        } else {
            " Form "
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(Span::styled(title, Style::default().fg(if is_active { theme::ACCENT } else { theme::DIM }).add_modifier(Modifier::BOLD)))
            .style(Style::default().bg(theme::BG));
        let inner = block.inner(area);
        f.render_widget(block, area);

        let mut rows: Vec<Row> = Vec::new();
        for (i, &field) in vis.iter().enumerate() {
            let is_focused = is_active && i == self.focus;
            let is_editing = is_focused && self.editing;

            let indicator = if is_editing {
                Span::styled(" > ", Style::default().fg(theme::GREEN).add_modifier(Modifier::BOLD))
            } else if is_focused {
                Span::styled(" > ", Style::default().fg(theme::ACCENT))
            } else {
                Span::styled("   ", Style::default().fg(theme::MUTED))
            };

            let label_style = if is_editing {
                Style::default().fg(theme::GREEN).add_modifier(Modifier::BOLD)
            } else if is_focused {
                Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::DIM)
            };

            let val = self.get(field);

            // Validate field and determine error state
            let field_error: Option<&str> = match field {
                Field::Time if !val.is_empty() => {
                    parse_time(&val).err().map(|_| "Invalid format (HH:MM:SS)")
                }
                Field::Memory if !val.is_empty() => {
                    parse_memory(&val).err().map(|_| "Invalid format (e.g. 4G)")
                }
                Field::Name if !val.is_empty() => {
                    validate_job_name(&val).err().map(|_| "Invalid job name")
                }
                Field::Nodes | Field::Ntasks | Field::Cpus if !val.is_empty() => {
                    if val.parse::<u32>().is_err() {
                        Some("Must be a number")
                    } else {
                        None
                    }
                }
                Field::Gpus if !val.is_empty() => {
                    if val.parse::<u32>().is_err() {
                        Some("Must be a number")
                    } else {
                        None
                    }
                }
                _ => None,
            };
            let has_error = field_error.is_some();

            // Build display string and row height
            let (display, row_height) = match field {
                Field::Mode => {
                    if self.mode_is_srun {
                        ("< srun >".to_string(), 1)
                    } else {
                        ("< sbatch >".to_string(), 1)
                    }
                }
                Field::Partition => {
                    if val.is_empty() {
                        ("< select >".to_string(), 1)
                    } else {
                        (format!("< {val} >"), 1)
                    }
                }
                Field::Modules | Field::Env | Field::Init => {
                    // Multiline fields: show a clean summary, Enter opens right-pane editor
                    if val.is_empty() {
                        let hint = if is_focused { "Enter to edit" } else { "\u{2014}" };
                        (hint.to_string(), 1)
                    } else {
                        let items: Vec<&str> = val.lines()
                            .map(|l| l.trim())
                            .filter(|l| !l.is_empty())
                            .collect();
                        let count = items.len();
                        let label = match field {
                            Field::Modules => if count == 1 { "module" } else { "modules" },
                            Field::Env => if count == 1 { "var" } else { "vars" },
                            Field::Init => if count == 1 { "cmd" } else { "cmds" },
                            _ => unreachable!(),
                        };
                        // Show up to 3 items inline, comma-separated
                        let shown: Vec<&str> = items.iter().take(3).copied().collect();
                        let summary = shown.join(", ");
                        if count > 3 {
                            (format!("{summary} (+{} {label})", count - 3), 1)
                        } else {
                            (format!("{summary}  [{count} {label}]"), 1)
                        }
                    }
                }
                _ => {
                    if val.is_empty() && !is_editing {
                        ("\u{2014}".to_string(), 1)
                    } else if is_editing {
                        let cursor_pos = self.field_cursor.min(val.len());
                        let mut display_val = val.clone();
                        display_val.insert(cursor_pos, '\u{2588}');
                        if has_error {
                            (format!("{display_val}  {}", field_error.unwrap()), 1)
                        } else {
                            (display_val, 1)
                        }
                    } else if has_error && is_focused {
                        (format!("{val}  {}", field_error.unwrap()), 1)
                    } else {
                        (val.clone(), 1)
                    }
                }
            };

            let val_style = if has_error && !val.is_empty() {
                if is_editing {
                    Style::default().fg(theme::RED).bg(theme::SURFACE)
                } else {
                    Style::default().fg(theme::RED)
                }
            } else if is_editing {
                Style::default().fg(theme::TEXT).bg(theme::SURFACE)
            } else if is_focused {
                Style::default().fg(theme::TEXT)
            } else {
                Style::default().fg(theme::MUTED)
            };

            rows.push(Row::new(vec![
                Cell::from(indicator),
                Cell::from(field.label().to_string()).style(label_style),
                Cell::from(format!(" {display}")).style(val_style),
            ]).height(row_height));
        }

        // Extra params section
        if !self.extra_params.is_empty() {
            rows.push(Row::new(vec![
                Cell::from(""),
                Cell::from("").style(Style::default().fg(theme::BORDER)),
                Cell::from(" -- Extra Parameters --").style(Style::default().fg(theme::MUTED)),
            ]));
        }
        for (ei, (key, value)) in self.extra_params.iter().enumerate() {
            let extra_focus_idx = vis.len() + ei;
            let is_focused = is_active && extra_focus_idx == self.focus;
            let is_editing = is_focused && self.editing;

            let indicator = if is_editing {
                Span::styled(" > ", Style::default().fg(theme::GREEN).add_modifier(Modifier::BOLD))
            } else if is_focused {
                Span::styled(" > ", Style::default().fg(theme::ACCENT))
            } else {
                Span::styled("   ", Style::default().fg(theme::MUTED))
            };

            let label = param_catalog::lookup(key)
                .map(|p| p.label)
                .unwrap_or(key.as_str());

            let label_style = if is_editing {
                Style::default().fg(theme::GREEN).add_modifier(Modifier::BOLD)
            } else if is_focused {
                Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme::DIM)
            };

            let is_flag = param_catalog::lookup(key).is_some_and(|p| p.is_flag);
            let display = if is_flag {
                "(flag)".to_string()
            } else if is_editing {
                let cursor_pos = self.field_cursor.min(value.len());
                let mut display_val = value.clone();
                display_val.insert(cursor_pos, '\u{2588}');
                display_val
            } else if value.is_empty() {
                "\u{2014}".to_string()
            } else {
                value.clone()
            };

            let val_style = if is_editing {
                Style::default().fg(theme::TEXT).bg(theme::SURFACE)
            } else if is_focused {
                Style::default().fg(theme::TEXT)
            } else {
                Style::default().fg(theme::MUTED)
            };

            rows.push(Row::new(vec![
                Cell::from(indicator),
                Cell::from(label.to_string()).style(label_style),
                Cell::from(format!(" {display}")).style(val_style),
            ]));
        }

        let table = Table::new(
            rows,
            [Constraint::Length(3), Constraint::Length(12), Constraint::Min(10)],
        )
        .block(Block::default());

        f.render_widget(table, inner);
    }

    fn draw_preview(&self, f: &mut Frame, area: Rect) {
        let is_active = self.active_pane == Pane::Preview;
        let is_editing = is_active && self.editing;

        let border_color = if is_editing {
            theme::GREEN
        } else if is_active {
            theme::ACCENT
        } else {
            theme::BORDER
        };

        let title = if is_editing {
            " Editor [editing] "
        } else if self.preview_dirty {
            " Editor [modified] "
        } else {
            " Preview "
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(Span::styled(title, Style::default().fg(if is_editing { theme::GREEN } else { theme::ACCENT }).add_modifier(Modifier::BOLD)))
            .style(Style::default().bg(theme::BG));

        let inner = block.inner(area);
        f.render_widget(block, area);

        // Use preview_text if available, else generate from form
        let text = if self.preview_text.is_empty() && !self.preview_dirty {
            self.generate_preview()
        } else {
            self.preview_text.clone()
        };

        if is_editing {
            // Show text with cursor indicator and line numbers
            let lines: Vec<&str> = text.split('\n').collect();
            let total_lines = lines.len();
            let gutter_width = if total_lines >= 100 { 5_u16 } else { 4 };

            // Find cursor position in terms of line/col
            let mut chars_counted = 0;
            let mut cursor_line = 0;
            let mut cursor_col = 0;
            for (li, line) in text.split('\n').enumerate() {
                if chars_counted + line.len() >= self.preview_cursor && self.preview_cursor >= chars_counted {
                    cursor_line = li;
                    cursor_col = self.preview_cursor - chars_counted;
                    break;
                }
                chars_counted += line.len() + 1; // +1 for newline
                cursor_line = li + 1;
            }

            // Auto-scroll to keep cursor visible
            let visible_height = inner.height as usize;
            let scroll = if cursor_line >= self.preview_scroll + visible_height {
                cursor_line - visible_height + 1
            } else if cursor_line < self.preview_scroll {
                cursor_line
            } else {
                self.preview_scroll
            };

            let end = (scroll + visible_height).min(lines.len());
            let visible_lines = &lines[scroll..end];

            // Split inner area into gutter + code
            let editor_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(gutter_width), Constraint::Min(0)])
                .split(inner);

            // Render line numbers in gutter
            let gutter_lines: Vec<Line> = visible_lines
                .iter()
                .enumerate()
                .map(|(vi, _)| {
                    let line_num = scroll + vi + 1;
                    let is_cursor = scroll + vi == cursor_line;
                    let style = if is_cursor {
                        Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme::MUTED)
                    };
                    Line::from(Span::styled(
                        format!("{:>width$} ", line_num, width = (gutter_width - 1) as usize),
                        style,
                    ))
                })
                .collect();
            f.render_widget(Paragraph::new(gutter_lines), editor_chunks[0]);

            // Render code with cursor
            let styled_lines: Vec<Line> = visible_lines
                .iter()
                .enumerate()
                .map(|(vi, line)| {
                    let actual_line = scroll + vi;
                    if actual_line == cursor_line {
                        let col = cursor_col.min(line.len());
                        let before = &line[..col];
                        let cursor_char = if col < line.len() {
                            &line[col..col + line[col..].chars().next().map(|c| c.len_utf8()).unwrap_or(1)]
                        } else {
                            " "
                        };
                        let after = if col < line.len() {
                            let skip = line[col..].chars().next().map(|c| c.len_utf8()).unwrap_or(0);
                            &line[col + skip..]
                        } else {
                            ""
                        };
                        Line::from(vec![
                            Span::styled(before.to_string(), Style::default().fg(theme::TEAL)),
                            Span::styled(cursor_char.to_string(), Style::default().fg(theme::BG).bg(theme::TEXT)),
                            Span::styled(after.to_string(), Style::default().fg(theme::TEAL)),
                        ])
                    } else {
                        Line::from(Span::styled(line.to_string(), Style::default().fg(theme::TEAL)))
                    }
                })
                .collect();

            f.render_widget(
                Paragraph::new(styled_lines),
                editor_chunks[1],
            );
        } else {
            // Read-only view
            let lines: Vec<&str> = text.split('\n').collect();
            let visible_height = inner.height as usize;
            let scroll = self.preview_scroll.min(lines.len().saturating_sub(visible_height));
            let end = (scroll + visible_height).min(lines.len());
            let visible_lines = &lines[scroll..end];

            let styled_lines: Vec<Line> = visible_lines
                .iter()
                .map(|line| Line::from(Span::styled(line.to_string(), Style::default().fg(theme::TEAL))))
                .collect();

            f.render_widget(
                Paragraph::new(styled_lines).wrap(Wrap { trim: false }),
                inner,
            );
        }
    }

    fn draw_field_editor(&self, f: &mut Frame, area: Rect, fe: &FieldEditor) {
        let title = format!(" {} [editing] ", fe.field.label());
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme::GREEN))
            .title(Span::styled(title, Style::default().fg(theme::GREEN).add_modifier(Modifier::BOLD)))
            .style(Style::default().bg(theme::BG));

        let inner = block.inner(area);
        f.render_widget(block, area);

        let text = &fe.text;
        let lines: Vec<&str> = text.split('\n').collect();
        let total_lines = lines.len();
        let gutter_width: u16 = if total_lines >= 100 { 5 } else { 4 };

        // Find cursor line/col
        let mut chars_counted = 0;
        let mut cursor_line = 0;
        let mut cursor_col = 0;
        for (li, line) in text.split('\n').enumerate() {
            if chars_counted + line.len() >= fe.cursor && fe.cursor >= chars_counted {
                cursor_line = li;
                cursor_col = fe.cursor - chars_counted;
                break;
            }
            chars_counted += line.len() + 1;
            cursor_line = li + 1;
        }

        // Auto-scroll
        let visible_height = inner.height as usize;
        let scroll = if cursor_line >= fe.scroll + visible_height {
            cursor_line - visible_height + 1
        } else if cursor_line < fe.scroll {
            cursor_line
        } else {
            fe.scroll
        };

        let end = (scroll + visible_height).min(lines.len());
        let visible_lines = &lines[scroll..end];

        // Split into gutter + code
        let editor_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(gutter_width), Constraint::Min(0)])
            .split(inner);

        // Render line numbers
        let gutter_lines: Vec<Line> = visible_lines
            .iter()
            .enumerate()
            .map(|(vi, _)| {
                let line_num = scroll + vi + 1;
                let is_cursor = scroll + vi == cursor_line;
                let style = if is_cursor {
                    Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme::MUTED)
                };
                Line::from(Span::styled(
                    format!("{:>width$} ", line_num, width = (gutter_width - 1) as usize),
                    style,
                ))
            })
            .collect();
        f.render_widget(Paragraph::new(gutter_lines), editor_chunks[0]);

        // Render code with cursor
        let styled_lines: Vec<Line> = visible_lines
            .iter()
            .enumerate()
            .map(|(vi, line)| {
                let actual_line = scroll + vi;
                if actual_line == cursor_line {
                    let col = cursor_col.min(line.len());
                    let before = &line[..col];
                    let cursor_char = if col < line.len() {
                        &line[col..col + line[col..].chars().next().map(|c| c.len_utf8()).unwrap_or(1)]
                    } else {
                        " "
                    };
                    let after = if col < line.len() {
                        let skip = line[col..].chars().next().map(|c| c.len_utf8()).unwrap_or(0);
                        &line[col + skip..]
                    } else {
                        ""
                    };
                    Line::from(vec![
                        Span::styled(before.to_string(), Style::default().fg(theme::TEAL)),
                        Span::styled(cursor_char.to_string(), Style::default().fg(theme::BG).bg(theme::TEXT)),
                        Span::styled(after.to_string(), Style::default().fg(theme::TEAL)),
                    ])
                } else {
                    Line::from(Span::styled(line.to_string(), Style::default().fg(theme::TEAL)))
                }
            })
            .collect();

        f.render_widget(Paragraph::new(styled_lines), editor_chunks[1]);
    }

    pub fn handle_mouse_click(&mut self, row: u16, col: u16, area: &Rect) {
        // Determine if click is in left pane (Form) or right pane (Preview)
        let half_width = area.width / 2;
        if col < half_width {
            // Form pane click
            self.active_pane = Pane::Form;
            self.editing = false;
            // Each field row corresponds to a row in the form (offset by block border)
            let vis = self.visible_fields();
            if row >= 1 {
                let idx = (row - 1) as usize;
                if idx < vis.len() {
                    self.focus = idx;
                }
            }
        } else {
            // Preview pane click
            self.active_pane = Pane::Preview;
            self.editing = false;
            if !self.preview_dirty {
                self.sync_preview_from_form();
            }
        }
    }

    pub fn scroll_down(&mut self) {
        match self.active_pane {
            Pane::Form => {
                let total = self.visible_fields().len() + self.extra_params.len();
                self.focus = (self.focus + 1).min(total.saturating_sub(1));
            }
            Pane::Preview => {
                let line_count = self.preview_text.lines().count().max(1);
                if self.preview_scroll + 1 < line_count {
                    self.preview_scroll += 1;
                }
            }
        }
    }

    pub fn scroll_up(&mut self) {
        match self.active_pane {
            Pane::Form => {
                self.focus = self.focus.saturating_sub(1);
            }
            Pane::Preview => {
                self.preview_scroll = self.preview_scroll.saturating_sub(1);
            }
        }
    }

    /// Get filtered catalog entries for the add-param dialog
    fn filtered_catalog_entries(&self, search: &str) -> Vec<usize> {
        let existing: Vec<&str> = self.extra_params.iter().map(|(k, _)| k.as_str()).collect();
        let search_lower = search.to_lowercase();
        param_catalog::ALL_PARAMS
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                // Exclude core params already in the form
                !param_catalog::CORE_PARAM_KEYS.contains(&p.key)
                    // Exclude already added extra params
                    && !existing.contains(&p.key)
                    // Filter by search
                    && (search.is_empty()
                        || p.key.to_lowercase().contains(&search_lower)
                        || p.label.to_lowercase().contains(&search_lower)
                        || p.short_desc.to_lowercase().contains(&search_lower))
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn draw_help_overlay(&self, f: &mut Frame, area: Rect) {
        let vis = self.visible_fields();
        let param_key = if self.focus < vis.len() {
            let field = vis[self.focus];
            param_catalog::form_key_to_param(Self::field_key(field))
        } else if self.focus - vis.len() < self.extra_params.len() {
            self.extra_params[self.focus - vis.len()].0.as_str()
        } else {
            return;
        };

        let entry = match param_catalog::lookup(param_key) {
            Some(e) => e,
            None => return,
        };

        let popup_area = centered_rect(60, 16, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(Span::styled(
                format!(" --{} ({}) ", entry.key, entry.label),
                Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD),
            ))
            .title_bottom(Span::styled(
                " Esc/? to close ",
                Style::default().fg(theme::MUTED),
            ))
            .border_style(Style::default().fg(theme::ACCENT))
            .style(Style::default().bg(theme::SURFACE));
        let inner = block.inner(popup_area);
        f.render_widget(Clear, popup_area);
        f.render_widget(block, popup_area);

        let mut lines: Vec<Line> = vec![
            Line::from(Span::styled(
                entry.short_desc,
                Style::default().fg(theme::LAVENDER).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
        ];
        for text_line in entry.long_desc.lines() {
            lines.push(Line::from(Span::styled(
                text_line.to_string(),
                Style::default().fg(theme::TEXT),
            )));
        }

        f.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            inner,
        );
    }

    fn draw_add_param_dialog(&self, f: &mut Frame, area: Rect, dialog: &AddParamDialog) {
        let filtered = self.filtered_catalog_entries(&dialog.search);
        let h = (filtered.len() as u16 + 5).min(20);
        let popup_area = centered_rect(55, h, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(Span::styled(
                " Add Parameter ",
                Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD),
            ))
            .title_bottom(Span::styled(
                " Type to search, Enter to add, Esc to cancel ",
                Style::default().fg(theme::MUTED),
            ))
            .border_style(Style::default().fg(theme::ACCENT))
            .style(Style::default().bg(theme::SURFACE));
        let inner = block.inner(popup_area);
        f.render_widget(Clear, popup_area);
        f.render_widget(block, popup_area);

        let search_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(0)])
            .split(inner);

        // Search input
        let cursor = if dialog.search.is_empty() { "" } else { "|" };
        f.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("  / ", Style::default().fg(theme::ACCENT)),
                Span::styled(
                    format!("{}{cursor}", dialog.search),
                    Style::default().fg(theme::TEXT),
                ),
            ])).style(Style::default().bg(theme::SURFACE)),
            search_chunks[0],
        );

        // List
        let visible_height = search_chunks[2].height as usize;
        let scroll = if dialog.selected >= visible_height {
            dialog.selected - visible_height + 1
        } else {
            0
        };
        let items: Vec<ListItem> = filtered
            .iter()
            .skip(scroll)
            .take(visible_height)
            .enumerate()
            .map(|(vi, &idx)| {
                let entry = &param_catalog::ALL_PARAMS[idx];
                let is_sel = scroll + vi == dialog.selected;
                let style = if is_sel {
                    Style::default().bg(theme::HIGHLIGHT).fg(theme::TEXT)
                } else {
                    Style::default().fg(theme::DIM)
                };
                let flag_mark = if entry.is_flag { " [flag]" } else { "" };
                ListItem::new(format!(
                    "  --{:<20} {}{}",
                    entry.key, entry.short_desc, flag_mark
                ))
                .style(style)
            })
            .collect();
        f.render_widget(List::new(items), search_chunks[2]);
    }

    fn draw_file_browser(&self, f: &mut Frame, area: Rect, fb: &FileBrowserDialog) {
        let h = area.height.clamp(12, 28);
        let popup_area = centered_rect(70, h, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(Span::styled(
                " Open Script ",
                Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD),
            ))
            .title_bottom(Span::styled(
                " Enter=open  Backspace=up  ~=home  Esc=cancel ",
                Style::default().fg(theme::MUTED),
            ))
            .border_style(Style::default().fg(theme::ACCENT))
            .style(Style::default().bg(theme::SURFACE));
        let inner = block.inner(popup_area);
        f.render_widget(Clear, popup_area);
        f.render_widget(block, popup_area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(2), Constraint::Min(0)])
            .split(inner);

        // Current directory path
        let dir_display = fb.current_dir.to_string_lossy();
        f.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled("  ", Style::default()),
                    Span::styled(dir_display.to_string(), Style::default().fg(theme::LAVENDER).add_modifier(Modifier::BOLD)),
                ]),
                Line::from(""),
            ]),
            chunks[0],
        );

        // Error message
        if let Some(ref err) = fb.error {
            f.render_widget(
                Paragraph::new(Span::styled(
                    format!("  {err}"),
                    Style::default().fg(theme::RED),
                )),
                chunks[1],
            );
            return;
        }

        // Empty directory
        if fb.entries.is_empty() {
            f.render_widget(
                Paragraph::new(Span::styled(
                    "  No scripts found (.sbatch, .sh, .job)",
                    Style::default().fg(theme::MUTED),
                )),
                chunks[1],
            );
            return;
        }

        // File list
        let visible_height = chunks[1].height as usize;
        let scroll = fb.scroll.min(fb.entries.len().saturating_sub(visible_height));
        let end = (scroll + visible_height).min(fb.entries.len());

        let items: Vec<ListItem> = fb.entries[scroll..end]
            .iter()
            .enumerate()
            .map(|(vi, entry)| {
                let actual_idx = scroll + vi;
                let is_sel = actual_idx == fb.selected;

                let (icon, name_style) = if entry.is_dir {
                    (" /", Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD))
                } else if entry.is_compatible {
                    (" *", Style::default().fg(theme::GREEN))
                } else {
                    ("  ", Style::default().fg(theme::MUTED))
                };

                let bg = if is_sel { theme::HIGHLIGHT } else { theme::SURFACE };
                let suffix = if !entry.is_dir && !entry.is_compatible {
                    Span::styled("  (not a valid script)", Style::default().fg(theme::MUTED).bg(bg))
                } else {
                    Span::raw("")
                };

                ListItem::new(Line::from(vec![
                    Span::styled(format!(" {icon} "), Style::default().fg(if entry.is_dir { theme::ACCENT } else if entry.is_compatible { theme::GREEN } else { theme::MUTED }).bg(bg)),
                    Span::styled(entry.name.clone(), name_style.bg(bg)),
                    suffix,
                ]))
            })
            .collect();

        f.render_widget(List::new(items), chunks[1]);
    }
}

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        out.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

