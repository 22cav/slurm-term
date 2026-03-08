use std::collections::HashMap;

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
    // Load .sbatch file dialog
    pub load_file_dialog: Option<String>,
}

pub struct AddParamDialog {
    pub search: String,
    pub selected: usize,
}

pub enum TemplateDialog {
    Save { name: String },
    Load { names: Vec<String>, selected: usize },
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
            load_file_dialog: None,
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

    fn generate_preview(&self) -> String {
        if self.mode_is_srun {
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
            parts.push("$SHELL".to_string());
            parts.join(" \\\n  ")
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

        // Load file dialog
        if let Some(ref mut path) = self.load_file_dialog {
            match key.code {
                KeyCode::Esc => {
                    self.load_file_dialog = None;
                }
                KeyCode::Enter => {
                    let p = path.clone();
                    self.load_file_dialog = None;
                    if p.is_empty() {
                        return Action::Status("No file path provided".into());
                    }
                    match sbatch_parser::parse_sbatch_file(&p) {
                        Ok(state) => {
                            self.set_form_state(&state);
                            return Action::Status(format!("Loaded {p}"));
                        }
                        Err(e) => {
                            return Action::Status(format!("! {e}"));
                        }
                    }
                }
                KeyCode::Backspace => { path.pop(); }
                KeyCode::Char(c) => { path.push(c); }
                _ => {}
            }
            return Action::None;
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
            let params = self.build_params();
            let script = self.get(Field::Script);
            if script.is_empty() && !self.mode_is_srun {
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

        // Ctrl+O: load .sbatch file
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('o') {
            self.load_file_dialog = Some(String::new());
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

        // Editing mode in preview - direct text editing
        match key.code {
            KeyCode::Esc => {
                self.editing = false;
                // Sync changes back to form
                if self.preview_dirty {
                    self.sync_form_from_preview();
                }
            }
            KeyCode::Char(c) if c != '\r' => {
                if self.preview_cursor <= self.preview_text.len() {
                    self.preview_text.insert(self.preview_cursor, c);
                    self.preview_cursor += c.len_utf8();
                    self.preview_dirty = true;
                }
            }
            KeyCode::Backspace => {
                if self.preview_cursor > 0 {
                    // Find previous char boundary
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
                self.preview_text.insert(self.preview_cursor, '\n');
                self.preview_cursor += 1;
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
                // Move to start of current line
                let before = &self.preview_text[..self.preview_cursor];
                self.preview_cursor = before.rfind('\n').map(|i| i + 1).unwrap_or(0);
            }
            KeyCode::End => {
                // Move to end of current line
                let after = &self.preview_text[self.preview_cursor..];
                self.preview_cursor += after.find('\n').unwrap_or(after.len());
            }
            KeyCode::Up => {
                // Move cursor up one line
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
                // Move cursor down one line
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

    pub fn draw(&self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);

        self.draw_form(f, chunks[0]);
        self.draw_preview(f, chunks[1]);

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

        // Load file dialog
        if let Some(ref path) = self.load_file_dialog {
            self.draw_load_file_dialog(f, f.area(), path);
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
            let is_focused = i == self.focus;
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
                    const MAX_VIS: usize = 8;
                    let line_count = if val.is_empty() { 1 } else {
                        val.lines().count() + if val.ends_with('\n') { 1 } else { 0 }
                    }.max(1);
                    if is_editing || (is_focused && line_count > 1) {
                        let vis_lines: Vec<&str> = if is_editing {
                            let cursor_pos = self.field_cursor.min(val.len());
                            // Determine which line the cursor is on
                            let cursor_line = val[..cursor_pos].matches('\n').count();
                            // Auto-scroll: handled via self.field_scroll
                            // (adjusted in key handler, but clamp here too)
                            let _ = cursor_line; // scroll already set
                            val.lines().collect()
                        } else {
                            val.lines().collect()
                        };
                        let scroll = self.field_scroll.min(vis_lines.len().saturating_sub(1));
                        let end = (scroll + MAX_VIS).min(vis_lines.len());
                        let window: Vec<&str> = vis_lines[scroll..end].to_vec();
                        let row_h = window.len().max(1) as u16;
                        let display_val = if is_editing {
                            // Rebuild with cursor block in the visible window
                            let cursor_pos = self.field_cursor.min(val.len());
                            let mut full = val.clone();
                            full.insert(cursor_pos, '\u{2588}');
                            let all_lines: Vec<&str> = full.lines().collect();
                            let end2 = (scroll + MAX_VIS).min(all_lines.len());
                            all_lines[scroll..end2].join("\n ")
                        } else {
                            window.join("\n ")
                        };
                        let scroll_hint = if line_count > MAX_VIS {
                            format!(" [{}/{} lines]", scroll + 1, line_count)
                        } else {
                            String::new()
                        };
                        (format!(" {display_val}{scroll_hint}"), row_h)
                    } else if line_count > 1 {
                        let first_line = val.lines().next().unwrap_or("");
                        (format!("{first_line} (+{} lines)", line_count - 1), 1)
                    } else if val.is_empty() && !is_editing {
                        ("\u{2014}".to_string(), 1)
                    } else {
                        (val.clone(), 1)
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
            let is_focused = extra_focus_idx == self.focus;
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
            " Preview [editing] "
        } else if self.preview_dirty {
            " Preview [modified] "
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
            // Show text with cursor indicator
            let lines: Vec<&str> = text.split('\n').collect();
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

            let styled_lines: Vec<Line> = visible_lines
                .iter()
                .enumerate()
                .map(|(vi, line)| {
                    let actual_line = scroll + vi;
                    if actual_line == cursor_line {
                        // Insert cursor marker
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
                inner,
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

    fn draw_load_file_dialog(&self, f: &mut Frame, area: Rect, path: &str) {
        let popup_area = centered_rect(50, 5, area);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .title(Span::styled(
                " Load .sbatch File ",
                Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD),
            ))
            .border_style(Style::default().fg(theme::ACCENT))
            .style(Style::default().bg(theme::SURFACE));
        let inner = block.inner(popup_area);
        f.render_widget(Clear, popup_area);
        f.render_widget(block, popup_area);
        f.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled("Path: ", Style::default().fg(theme::DIM)),
                    Span::styled(format!("{path}\u{2588}"), Style::default().fg(theme::TEXT)),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "Enter to load, Esc to cancel",
                    Style::default().fg(theme::MUTED),
                )),
            ]),
            inner,
        );
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

