use std::collections::HashMap;
use std::path::PathBuf;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::*;
use tui_textarea::{CursorMove, TextArea};

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

#[derive(Clone, Copy, PartialEq, Debug)]
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
    // Editable preview, backed by the tui-textarea widget.
    preview: TextArea<'static>,
    preview_dirty: bool, // true = preview text was edited manually
    // Cursor position within the currently edited form field
    // Inline single-line editor for the focused form/extra field. The text
    // lives in the widget while editing and is committed back on exit.
    field_input: Option<TextArea<'static>>,
    // Scroll offset for multiline form fields (line index of first visible line)
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

/// Editor for a multiline field (Modules/Env/Init) shown in the right pane.
struct FieldEditor {
    field: Field,
    editor: TextArea<'static>,
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

/// Build a themed multi-line editor seeded with `text`.
fn styled_textarea(text: &str) -> TextArea<'static> {
    let mut ta = TextArea::new(text.split('\n').map(str::to_string).collect());
    ta.set_style(Style::default().fg(theme::TEAL));
    ta.set_cursor_line_style(Style::default()); // no full-width line highlight
    ta
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
            preview: styled_textarea(""),
            preview_dirty: false,
            field_input: None,
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
        self.absorb_extra_directives();
        self.mode_is_srun = self.fields.get("mode").is_some_and(|m| m == "srun");
        self.sync_preview_from_form();
    }

    /// Pull any `extra.<key>` directives (unknown #SBATCH options parsed from a
    /// script or the preview) out of `fields` and into `extra_params`, so they
    /// survive as visible form rows and reach the submitted command.
    fn absorb_extra_directives(&mut self) {
        let keys: Vec<String> = self
            .fields
            .keys()
            .filter(|k| k.starts_with("extra."))
            .cloned()
            .collect();
        for k in keys {
            let val = self.fields.remove(&k).unwrap_or_default();
            let name = k.trim_start_matches("extra.").to_string();
            if name.is_empty() {
                continue;
            }
            if let Some(e) = self.extra_params.iter_mut().find(|(ek, _)| *ek == name) {
                e.1 = val;
            } else {
                self.extra_params.push((name, val));
            }
        }
    }

    /// Current preview/editor contents as a single string.
    fn preview_string(&self) -> String {
        self.preview.lines().join("\n")
    }

    /// True when the preview holds no text (a single empty line).
    pub fn preview_is_empty(&self) -> bool {
        matches!(self.preview.lines(), [only] if only.is_empty())
    }

    /// True while the Modules/Env/Init popup editor is open.
    pub fn field_editor_open(&self) -> bool {
        self.field_editor.is_some()
    }

    /// Open the inline single-line editor on the focused field.
    fn start_field_edit(&mut self, initial: &str) {
        let mut ta = TextArea::new(vec![initial.to_string()]);
        ta.set_style(Style::default().fg(theme::TEXT).bg(theme::SURFACE));
        ta.set_cursor_line_style(Style::default().bg(theme::SURFACE));
        ta.set_cursor_style(Style::default().bg(theme::TEXT).fg(theme::BG));
        ta.move_cursor(CursorMove::End);
        self.field_input = Some(ta);
        self.editing = true;
    }

    /// Commit the inline editor's text back into its field and close it.
    /// Safe to call from any exit path (Esc, Enter, Tab, click, submit).
    fn commit_field_input(&mut self) {
        let Some(ta) = self.field_input.take() else { return };
        let text = ta.into_lines().join("");
        let vis = self.visible_fields();
        if self.focus < vis.len() {
            self.set(vis[self.focus], text);
        } else if let Some((_, v)) = self.extra_params.get_mut(self.focus - vis.len()) {
            *v = text;
        }
        self.editing = false;
        self.sync_preview_from_form();
    }

    /// Save the Modules/Env/Init popup editor back to its field and close it.
    fn close_field_editor(&mut self) {
        let Some(fe) = self.field_editor.take() else { return };
        let text = fe.editor.lines().join("\n");
        self.set(fe.field, text);
        self.editing = false;
        self.sync_preview_from_form();
    }

    /// Regenerate preview text from form fields
    pub fn sync_preview_from_form(&mut self) {
        self.preview = styled_textarea(&self.generate_preview());
        self.preview_dirty = false;
    }

    /// Parse preview text back into form fields
    fn sync_form_from_preview(&mut self) {
        let text = self.preview_string();
        let parsed = sbatch_parser::parse_sbatch_text(&text);
        for (k, v) in &parsed {
            self.fields.insert(k.clone(), v.clone());
        }
        self.absorb_extra_directives();
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
        // Partition is intentionally optional: an empty --partition lets sbatch
        // use the cluster's default partition.
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
                                    self.preview.move_cursor(CursorMove::Bottom);
                                    self.preview.move_cursor(CursorMove::End);
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
            // A field being typed right now must reach the submission.
            self.commit_field_input();
            let was_dirty = self.preview_dirty;
            // If preview was manually edited, sync back to form first
            if was_dirty {
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
            // Submit the existing script file directly only when there is
            // nothing else to inject; otherwise the generated body (which the
            // preview shows) carries the module/env/init setup and the script
            // path as its final command.
            let has_setup = [Field::Modules, Field::Env, Field::Init]
                .iter()
                .any(|&f| !self.get(f).trim().is_empty());
            if !script.is_empty() && !has_setup && !was_dirty {
                return Action::Submit(params, script, String::new());
            }
            let body = if was_dirty {
                self.preview_string()
            } else {
                self.generate_preview()
            };
            return Action::Submit(params, String::new(), body);
        }

        // Ctrl+T: save template
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('t') {
            self.commit_field_input();
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

        // Ctrl+G: copy preview to clipboard (OSC 52). Not Ctrl+Y — that is the
        // text editor's paste binding, which the editor must keep.
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('g') {
            use std::io::Write;
            let encoded = base64_encode(self.preview_string().as_bytes());
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
                }
                KeyCode::BackTab | KeyCode::Up | KeyCode::Char('k') => {
                    self.focus = if self.focus == 0 { total_fields.saturating_sub(1) } else { self.focus - 1 };
                }
                KeyCode::Enter | KeyCode::Char(' ') => {
                    if in_extra {
                        let entry_key = self.extra_params[self.focus - vis.len()].0.clone();
                        if !param_catalog::lookup(&entry_key).is_some_and(|p| p.is_flag) {
                            let v = self.extra_params[self.focus - vis.len()].1.clone();
                            self.start_field_edit(&v);
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
                                let mut editor = styled_textarea(&self.get(field));
                                editor.move_cursor(CursorMove::Bottom);
                                editor.move_cursor(CursorMove::End);
                                self.field_editor = Some(FieldEditor { field, editor });
                                self.editing = true;
                            }
                            _ => {
                                let v = self.get(field);
                                self.start_field_edit(&v);
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

        // When EDITING: the inline TextArea owns the text; commit on every
        // exit path so nothing typed is ever lost.
        match key.code {
            KeyCode::Esc => {
                self.commit_field_input();
            }
            KeyCode::Enter | KeyCode::Tab => {
                self.commit_field_input();
                self.focus = (self.focus + 1) % total_fields;
            }
            KeyCode::BackTab => {
                self.commit_field_input();
                self.focus = if self.focus == 0 { total_fields.saturating_sub(1) } else { self.focus - 1 };
            }
            // Ctrl+M would insert a newline into a single-line field
            KeyCode::Char('m') if key.modifiers.contains(KeyModifiers::CONTROL) => {}
            _ => {
                if let Some(ta) = self.field_input.as_mut() {
                    ta.input(key);
                }
            }
        }
        Action::None
    }

    /// Handle pasted text (from bracketed paste mode).
    /// Normalizes CRLF → LF and strips stray CR before inserting.
    pub fn handle_paste(&mut self, text: &str) {
        let clean = text.replace("\r\n", "\n").replace('\r', "\n");
        if let Some(fe) = self.field_editor.as_mut() {
            fe.editor.insert_str(&clean);
        } else if self.active_pane == Pane::Preview && self.editing {
            self.preview.insert_str(&clean);
            self.preview_dirty = true;
        } else if let Some(ta) = self.field_input.as_mut() {
            // Inline form/extra fields are single-line; drop newlines.
            ta.insert_str(clean.replace('\n', ""));
        }
    }

    fn handle_key_preview(&mut self, key: KeyEvent) -> Action {
        if !self.editing {
            // Navigation mode: scroll the read-only view or hand off the pane.
            match key.code {
                KeyCode::Tab | KeyCode::BackTab | KeyCode::Esc => {
                    if self.preview_dirty {
                        self.sync_form_from_preview();
                    }
                    self.active_pane = Pane::Form;
                }
                KeyCode::Enter => {
                    self.editing = true;
                    self.preview.move_cursor(CursorMove::Bottom);
                    self.preview.move_cursor(CursorMove::End);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.preview.scroll((1, 0));
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.preview.scroll((-1, 0));
                }
                _ => {}
            }
            return Action::None;
        }

        // Editing mode: Esc leaves; everything else drives the text editor
        // (standard tui-textarea bindings, including Ctrl+Z undo).
        if key.code == KeyCode::Esc {
            self.editing = false;
            if self.preview_dirty {
                self.sync_form_from_preview();
            }
        } else if self.preview.input(key) {
            self.preview_dirty = true;
        }
        Action::None
    }

    /// Handle keys when the multiline field editor is active.
    /// Reuses the same editing logic as the preview editor.
    /// Drive the multiline field editor. Esc saves back to the form field and
    /// closes; every other key uses standard tui-textarea editing.
    fn handle_key_field_editor(&mut self, key: KeyEvent) -> Action {
        if key.code == KeyCode::Esc {
            self.close_field_editor();
        } else if let Some(fe) = self.field_editor.as_mut() {
            fe.editor.input(key);
        }
        Action::None
    }

    pub fn draw(&mut self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);

        self.draw_form(f, chunks[0]);

        // Right pane: field editor takes priority over preview
        if self.field_editor.is_some() {
            self.draw_field_editor(f, chunks[1]);
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
                    if is_editing {
                        // The inline TextArea overlay draws the live text.
                        (String::new(), 1)
                    } else if val.is_empty() {
                        ("\u{2014}".to_string(), 1)
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
                // The inline TextArea overlay draws the live text.
                String::new()
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

        // Inline single-line editor: overlay the focused row's value cell.
        // Column x offset = indicator(3) + spacing + label(12) + spacing + pad.
        if let Some(ta) = self.field_input.as_ref() {
            let row = if self.focus < vis.len() {
                self.focus
            } else {
                vis.len() + 1 + (self.focus - vis.len()) // +1 skips separator
            } as u16;
            const VALUE_X: u16 = 18;
            if row < inner.height && VALUE_X < inner.width {
                let rect = Rect::new(
                    inner.x + VALUE_X,
                    inner.y + row,
                    inner.width - VALUE_X,
                    1,
                );
                f.render_widget(ta, rect);
            }
        }
    }

    fn draw_preview(&mut self, f: &mut Frame, area: Rect) {
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
            .title(Span::styled(
                title,
                Style::default()
                    .fg(if is_editing { theme::GREEN } else { theme::ACCENT })
                    .add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(theme::BG));

        // Show the cursor and line numbers only while editing.
        if is_editing {
            self.preview
                .set_cursor_style(Style::default().bg(theme::TEXT).fg(theme::BG));
            self.preview
                .set_line_number_style(Style::default().fg(theme::MUTED));
        } else {
            self.preview.set_cursor_style(Style::default());
            self.preview.remove_line_number();
        }
        self.preview.set_block(block);
        f.render_widget(&self.preview, area);
    }

    fn draw_field_editor(&mut self, f: &mut Frame, area: Rect) {
        let Some(fe) = self.field_editor.as_mut() else { return };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme::GREEN))
            .title(Span::styled(
                format!(" {} [editing] ", fe.field.label()),
                Style::default().fg(theme::GREEN).add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(theme::BG));
        fe.editor
            .set_cursor_style(Style::default().bg(theme::TEXT).fg(theme::BG));
        fe.editor
            .set_line_number_style(Style::default().fg(theme::MUTED));
        fe.editor.set_block(block);
        f.render_widget(&fe.editor, area);
    }

    pub fn handle_mouse_click(&mut self, row: u16, col: u16, area: &Rect) {
        // Commit any in-progress edit before focus moves — a click must never
        // silently discard typed text.
        self.close_field_editor();
        self.commit_field_input();

        // Determine if click is in left pane (Form) or right pane (Preview)
        let half_width = area.width / 2;
        if col < half_width {
            // Form pane click
            self.active_pane = Pane::Form;
            self.editing = false;
            if self.preview_dirty {
                self.sync_form_from_preview();
            }
            // Rows are offset by the block's top border. Core fields occupy
            // rows 1..=vis.len(); a "-- Extra Parameters --" separator then sits
            // between them and the extra-param rows.
            let vis = self.visible_fields();
            if row >= 1 {
                let idx = (row - 1) as usize;
                if idx < vis.len() {
                    self.focus = idx;
                } else if !self.extra_params.is_empty() && idx > vis.len() {
                    let extra_idx = idx - vis.len() - 1; // skip the separator row
                    if extra_idx < self.extra_params.len() {
                        self.focus = vis.len() + extra_idx;
                    }
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
                self.preview.scroll((1, 0));
            }
        }
    }

    pub fn scroll_up(&mut self) {
        match self.active_pane {
            Pane::Form => {
                self.focus = self.focus.saturating_sub(1);
            }
            Pane::Preview => {
                self.preview.scroll((-1, 0));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock_slurm::MockSlurmController;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn type_str(c: &mut ComposerState, slurm: &dyn SlurmController, s: &str) {
        for ch in s.chars() {
            c.handle_key(key(KeyCode::Char(ch)), slurm);
        }
    }

    fn focus_field(c: &mut ComposerState, f: Field) {
        c.focus = Field::all().iter().position(|x| *x == f).unwrap();
    }

    // -----------------------------------------------------------------------
    // Single-line form fields (inline TextArea)
    // -----------------------------------------------------------------------

    #[test]
    fn typing_is_buffered_and_committed_on_esc() {
        let slurm = MockSlurmController::new(0, Some(1));
        let mut c = ComposerState::new();
        focus_field(&mut c, Field::Name);
        c.handle_key(key(KeyCode::Enter), &slurm); // start editing "my_job"
        type_str(&mut c, &slurm, "2");
        assert_eq!(c.get(Field::Name), "my_job", "not committed while typing");
        c.handle_key(key(KeyCode::Esc), &slurm);
        assert_eq!(c.get(Field::Name), "my_job2");
        assert!(!c.editing, "Esc must leave editing mode");
    }

    #[test]
    fn enter_commits_and_advances_to_next_field() {
        let slurm = MockSlurmController::new(0, Some(1));
        let mut c = ComposerState::new();
        focus_field(&mut c, Field::Name);
        let start_focus = c.focus;
        c.handle_key(key(KeyCode::Enter), &slurm);
        type_str(&mut c, &slurm, "x");
        c.handle_key(key(KeyCode::Enter), &slurm);
        assert_eq!(c.get(Field::Name), "my_jobx");
        assert_eq!(c.focus, start_focus + 1);
        assert!(!c.editing);
    }

    #[test]
    fn editor_bindings_work_in_single_line_fields() {
        let slurm = MockSlurmController::new(0, Some(1));
        let mut c = ComposerState::new();
        focus_field(&mut c, Field::Name);
        c.handle_key(key(KeyCode::Enter), &slurm);
        c.handle_key(ctrl('a'), &slurm); // line start
        c.handle_key(ctrl('k'), &slurm); // kill to end of line
        c.handle_key(key(KeyCode::Esc), &slurm);
        assert_eq!(c.get(Field::Name), "", "Ctrl+A then Ctrl+K must clear the field");
    }

    #[test]
    fn undo_works_in_single_line_fields() {
        let slurm = MockSlurmController::new(0, Some(1));
        let mut c = ComposerState::new();
        focus_field(&mut c, Field::Name);
        c.handle_key(key(KeyCode::Enter), &slurm);
        type_str(&mut c, &slurm, "z");
        c.handle_key(ctrl('u'), &slurm); // undo
        c.handle_key(key(KeyCode::Esc), &slurm);
        assert_eq!(c.get(Field::Name), "my_job", "Ctrl+U must undo the insertion");
    }

    #[test]
    fn newlines_cannot_enter_single_line_fields() {
        let slurm = MockSlurmController::new(0, Some(1));
        let mut c = ComposerState::new();
        focus_field(&mut c, Field::Name);
        c.handle_key(key(KeyCode::Enter), &slurm);
        c.handle_key(ctrl('m'), &slurm); // would insert newline in the widget
        c.handle_paste("ab\ncd"); // paste with embedded newline
        c.handle_key(key(KeyCode::Esc), &slurm);
        let v = c.get(Field::Name);
        assert!(!v.contains('\n'), "single-line value contains newline: {v:?}");
        assert_eq!(v, "my_jobabcd");
    }

    #[test]
    fn extra_param_edits_through_the_same_editor() {
        let slurm = MockSlurmController::new(0, Some(1));
        let mut c = ComposerState::new();
        c.extra_params.push(("account".into(), String::new()));
        c.focus = c.visible_fields().len(); // first extra param
        c.handle_key(key(KeyCode::Enter), &slurm);
        type_str(&mut c, &slurm, "proj");
        c.handle_key(key(KeyCode::Esc), &slurm);
        assert_eq!(c.extra_params[0].1, "proj");
        assert!(c.generate_preview().contains("--account=proj"));
    }

    #[test]
    fn ctrl_s_commits_the_field_being_typed() {
        let slurm = MockSlurmController::new(0, Some(1));
        let mut c = ComposerState::new();
        c.set(Field::Script, "/home/me/run.sh".into());
        focus_field(&mut c, Field::Name);
        c.handle_key(key(KeyCode::Enter), &slurm);
        type_str(&mut c, &slurm, "2");
        let action = c.handle_key(ctrl('s'), &slurm);
        match action {
            Action::Submit(params, script, _) => {
                assert_eq!(params.get("job-name"), Some(&"my_job2".to_string()));
                assert_eq!(script, "/home/me/run.sh");
            }
            _ => panic!("expected Submit action"),
        }
    }

    // -----------------------------------------------------------------------
    // Preview editor
    // -----------------------------------------------------------------------

    #[test]
    fn typing_in_the_preview_edits_the_text_and_marks_it_dirty() {
        let slurm = MockSlurmController::new(0, Some(1));
        let mut c = ComposerState::new();
        c.sync_preview_from_form();
        c.active_pane = Pane::Preview;
        c.editing = true;
        type_str(&mut c, &slurm, "XZ");
        assert!(c.preview_dirty);
        assert!(c.preview_string().contains("XZ"));
    }

    #[test]
    fn editor_bindings_reach_the_preview_widget() {
        let slurm = MockSlurmController::new(0, Some(1));
        let mut c = ComposerState::new();
        c.sync_preview_from_form(); // first line is "#!/bin/bash", cursor (0,0)
        c.active_pane = Pane::Preview;
        c.editing = true;
        c.handle_key(ctrl('k'), &slurm); // kill first line's content
        assert!(c.preview_dirty);
        assert!(!c.preview.lines()[0].contains("#!/bin/bash"));
    }

    #[test]
    fn esc_exits_the_preview_editor() {
        let slurm = MockSlurmController::new(0, Some(1));
        let mut c = ComposerState::new();
        c.active_pane = Pane::Preview;
        c.editing = true;
        c.handle_key(key(KeyCode::Esc), &slurm);
        assert!(!c.editing);
    }

    #[test]
    fn esc_in_preview_navigation_returns_to_form_pane() {
        let slurm = MockSlurmController::new(0, Some(1));
        let mut c = ComposerState::new();
        c.active_pane = Pane::Preview;
        c.editing = false;
        c.handle_key(key(KeyCode::Esc), &slurm);
        assert_eq!(c.active_pane, Pane::Form);
    }

    #[test]
    fn ctrl_s_submits_from_the_preview_editor_when_valid() {
        let slurm = MockSlurmController::new(0, Some(1));
        let mut c = ComposerState::new();
        c.set(Field::Partition, "batch".into());
        c.set(Field::Script, "/home/me/run.sh".into());
        c.sync_preview_from_form();
        c.active_pane = Pane::Preview;
        c.editing = true;
        let action = c.handle_key(ctrl('s'), &slurm);
        assert!(matches!(action, Action::Submit(..)));
    }

    // -----------------------------------------------------------------------
    // Popup field editor (Modules/Env/Init)
    // -----------------------------------------------------------------------

    #[test]
    fn field_editor_opens_and_saves_back_to_the_form_field() {
        let slurm = MockSlurmController::new(0, Some(1));
        let mut c = ComposerState::new();
        focus_field(&mut c, Field::Modules);
        c.handle_key(key(KeyCode::Enter), &slurm);
        assert!(c.field_editor.is_some());
        assert!(c.editing, "popup must count as editing (blocks q-quit)");
        type_str(&mut c, &slurm, "cuda");
        c.handle_key(key(KeyCode::Esc), &slurm);
        assert!(c.field_editor.is_none());
        assert!(!c.editing);
        assert_eq!(c.get(Field::Modules), "cuda");
    }

    #[test]
    fn field_editor_supports_multiline_and_undo() {
        let slurm = MockSlurmController::new(0, Some(1));
        let mut c = ComposerState::new();
        focus_field(&mut c, Field::Modules);
        c.handle_key(key(KeyCode::Enter), &slurm);
        type_str(&mut c, &slurm, "cuda");
        c.handle_key(key(KeyCode::Enter), &slurm); // newline stays in popup
        type_str(&mut c, &slurm, "python");
        c.handle_key(ctrl('u'), &slurm); // undo last insertion
        c.handle_key(key(KeyCode::Esc), &slurm);
        let v = c.get(Field::Modules);
        assert!(v.starts_with("cuda\n"), "newline lost: {v:?}");
        assert!(!v.contains("python"), "undo did not revert: {v:?}");
    }

    // -----------------------------------------------------------------------
    // Mouse: clicks must commit, never discard
    // -----------------------------------------------------------------------

    #[test]
    fn clicking_commits_the_inline_field_edit() {
        let slurm = MockSlurmController::new(0, Some(1));
        let mut c = ComposerState::new();
        focus_field(&mut c, Field::Name);
        c.handle_key(key(KeyCode::Enter), &slurm);
        type_str(&mut c, &slurm, "9");
        c.handle_mouse_click(1, 1, &Rect::new(0, 0, 100, 30));
        assert_eq!(c.get(Field::Name), "my_job9", "click discarded typed text");
        assert!(!c.editing);
    }

    #[test]
    fn clicking_commits_the_field_editor_popup() {
        let slurm = MockSlurmController::new(0, Some(1));
        let mut c = ComposerState::new();
        focus_field(&mut c, Field::Modules);
        c.handle_key(key(KeyCode::Enter), &slurm);
        type_str(&mut c, &slurm, "cuda");
        c.handle_mouse_click(2, 1, &Rect::new(0, 0, 100, 30));
        assert!(c.field_editor.is_none());
        assert_eq!(c.get(Field::Modules), "cuda");
    }

    #[test]
    fn clicking_into_the_form_applies_preview_edits() {
        let slurm = MockSlurmController::new(0, Some(1));
        let mut c = ComposerState::new();
        c.sync_preview_from_form();
        c.active_pane = Pane::Preview;
        c.editing = true;
        // Append a new directive on its own line at the end of line 0
        c.handle_key(key(KeyCode::End), &slurm);
        c.handle_key(key(KeyCode::Enter), &slurm);
        type_str(&mut c, &slurm, "#SBATCH --account=abc");
        assert!(c.preview_dirty);
        // Click into the form pane: the edit must be parsed into the form
        c.handle_mouse_click(3, 1, &Rect::new(0, 0, 100, 30));
        assert!(
            c.extra_params.iter().any(|(k, v)| k == "account" && v == "abc"),
            "preview edit lost on click: {:?}",
            c.extra_params
        );
        assert!(!c.preview_dirty);
    }

    // -----------------------------------------------------------------------
    // Form <-> preview round trips
    // -----------------------------------------------------------------------

    #[test]
    fn field_commit_regenerates_the_preview() {
        let slurm = MockSlurmController::new(0, Some(1));
        let mut c = ComposerState::new();
        focus_field(&mut c, Field::Name);
        c.handle_key(key(KeyCode::Enter), &slurm);
        type_str(&mut c, &slurm, "_new");
        c.handle_key(key(KeyCode::Esc), &slurm);
        assert!(c.preview_string().contains("--job-name=my_job_new"));
    }

    #[test]
    fn unknown_directives_survive_load_and_reach_params() {
        let mut c = ComposerState::new();
        let state = sbatch_parser::parse_sbatch_text(
            "#!/bin/bash\n#SBATCH --account=proj\n#SBATCH --partition=gpu\necho hi\n",
        );
        c.set_form_state(&state);
        assert!(c.extra_params.iter().any(|(k, v)| k == "account" && v == "proj"));
        assert_eq!(c.build_params().get("account"), Some(&"proj".to_string()));
        assert!(c.generate_preview().contains("--account=proj"));
    }

    #[test]
    fn extra_directives_are_not_duplicated_on_resync() {
        let mut c = ComposerState::new();
        let state = sbatch_parser::parse_sbatch_text("#SBATCH --account=proj\n");
        c.set_form_state(&state);
        c.sync_form_from_preview();
        assert_eq!(c.extra_params.iter().filter(|(k, _)| k == "account").count(), 1);
    }

    #[test]
    fn preview_carries_setup_commands_with_a_script_path() {
        let mut c = ComposerState::new();
        c.set(Field::Script, "/home/me/train.sh".into());
        c.set(Field::Modules, "cuda".into());
        c.set(Field::Init, "python train.py".into());
        let body = c.generate_preview();
        assert!(body.contains("module load cuda"));
        assert!(body.contains("python train.py"));
        assert!(body.contains("/home/me/train.sh"));
    }

    /// A key sbatch accepts as `--<key>`: a catalog entry or at least a
    /// multi-character long option name (never a bare short letter).
    fn is_valid_long_option(k: &str) -> bool {
        param_catalog::lookup(k).is_some()
            || (k.len() > 1
                && k.chars().next().is_some_and(|c| c.is_ascii_lowercase())
                && k.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
    }

    #[test]
    fn short_directives_never_emit_invalid_long_options() {
        let mut c = ComposerState::new();
        let state = sbatch_parser::parse_sbatch_text(
            "#!/bin/bash\n#SBATCH -n 4\n#SBATCH -A proj\n#SBATCH -q high\n#SBATCH -a 0-9%2\n\
             #SBATCH -d afterok:1\n#SBATCH -w node01\n#SBATCH -x node02\n#SBATCH -C a100\n\
             #SBATCH -G 2\n#SBATCH -H\necho hi\n",
        );
        c.set_form_state(&state);
        let params = c.build_params();
        for k in params.keys() {
            assert!(is_valid_long_option(k), "invalid option emitted: --{k}");
        }
        assert_eq!(params.get("ntasks"), Some(&"4".to_string()));
        assert_eq!(params.get("account"), Some(&"proj".to_string()));
        assert_eq!(params.get("array"), Some(&"0-9%2".to_string()));
        assert_eq!(params.get("gpus"), Some(&"2".to_string()));
        assert_eq!(params.get("hold"), Some(&String::new()));
        let preview = c.generate_preview();
        assert!(!preview.contains("--n="), "{preview}");
        assert!(preview.contains("#SBATCH --hold\n"), "{preview}");
    }

    #[test]
    fn preview_round_trips_through_the_parser() {
        let mut c = ComposerState::new();
        let state = sbatch_parser::parse_sbatch_text(
            "#!/bin/bash\n#SBATCH --job-name=rt\n#SBATCH --time=1-00:00:00\n#SBATCH --exclusive\n\
             #SBATCH --gres=gpu:a100:2\n#SBATCH --mail-type=END,FAIL\n\nmodule load cuda\npython x.py\n",
        );
        c.set_form_state(&state);
        let first = c.build_params();
        let preview = c.generate_preview();
        let mut c2 = ComposerState::new();
        c2.set_form_state(&sbatch_parser::parse_sbatch_text(&preview));
        assert_eq!(c2.build_params(), first);
        assert_eq!(first.get("gres"), Some(&"gpu:a100:2".to_string()));
        assert_eq!(first.get("exclusive"), Some(&String::new()));
    }

    #[test]
    fn unlimited_time_is_accepted() {
        let mut c = ComposerState::new();
        c.set(Field::Name, "job".into());
        c.set(Field::Init, "true".into());
        c.set(Field::Time, "UNLIMITED".into());
        assert_eq!(c.validate(), None);
    }

    #[test]
    fn test_only_submit_reports_estimate_instead_of_job_id() {
        let slurm = MockSlurmController::new(0, Some(1));
        let params = HashMap::from([("test-only".to_string(), String::new())]);
        match slurm.submit_job("x.sh", &params).unwrap() {
            crate::slurm_api::SubmitOutcome::TestOnly(msg) => assert!(msg.contains("to start at")),
            other => panic!("expected TestOnly, got {other:?}"),
        }
    }

    #[test]
    fn every_catalog_key_is_a_valid_long_option() {
        for p in param_catalog::ALL_PARAMS {
            assert!(is_valid_long_option(p.key), "{}", p.key);
            assert!(!p.short_desc.is_empty() && !p.long_desc.is_empty(), "{}", p.key);
        }
        let mut keys: Vec<_> = param_catalog::ALL_PARAMS.iter().map(|p| p.key).collect();
        keys.sort();
        let n = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), n, "duplicate catalog keys");
    }
}
