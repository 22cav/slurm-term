use std::collections::HashMap;
use std::io::BufRead;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::slurm_api::{job_state_of, slurm_val_to_string, SlurmController};
use crate::theme;
use crate::validators::{parse_rss_to_pct, parse_cpu_pct};

const METRICS_ROLLING_WINDOW: usize = 60;
const MEMORY_FALLBACK_MB: u64 = 64_000;
const LOG_TAIL_LINES: usize = 200;

/// Strip ANSI escape sequences and handle carriage returns to produce clean log lines.
fn sanitize_log_line(s: &str) -> String {
    // Handle carriage returns: simulate terminal behavior by keeping only the last segment.
    // e.g. "loading...\rDone!" -> "Done!"
    let visible = if s.contains('\r') {
        s.split('\r').next_back().unwrap_or(s)
    } else {
        s
    };

    // Strip ANSI/VT100 escape sequences.
    let mut result = String::with_capacity(visible.len());
    let mut chars = visible.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => {
                match chars.peek().copied() {
                    Some('[') => {
                        // CSI sequence: ESC [ ... <final>
                        chars.next(); // consume '['
                        for nc in chars.by_ref() {
                            if matches!(nc, 'A'..='Z' | 'a'..='z' | '~') {
                                break;
                            }
                        }
                    }
                    Some(']') => {
                        // OSC sequence: ESC ] ... BEL
                        chars.next(); // consume ']'
                        for nc in chars.by_ref() {
                            if nc == '\x07' {
                                break;
                            }
                        }
                    }
                    Some(c) if c.is_ascii_alphabetic() => {
                        chars.next(); // consume two-char escape (e.g. ESC M)
                    }
                    _ => {} // bare ESC, skip
                }
            }
            // Strip other non-printable control characters except tab
            '\x00'..='\x08' | '\x0b'..='\x0c' | '\x0e'..='\x1f' => {}
            _ => result.push(c),
        }
    }
    result
}

pub enum Action {
    None,
    Back,
    Refresh,
    Resubmit(HashMap<String, String>),
}

#[derive(Clone, Copy, PartialEq)]
enum SubTab {
    Overview,
    Logs,
    Metrics,
}

pub struct InspectorState {
    pub job_id: Option<String>,
    pub details: Option<serde_json::Value>,
    gpu_enabled: bool,
    sub_tab: SubTab,
    // Logs
    log_lines: Vec<String>,
    log_scroll: usize,
    log_mode: LogMode,
    pub log_follow: bool,
    log_last_len: u64, // track file size to skip re-reads when unchanged
    // Metrics
    cpu_history: Vec<f64>,
    mem_history: Vec<f64>,
    gpu_history: Vec<f64>,
}

#[derive(Clone, Copy, PartialEq)]
enum LogMode {
    Stdout,
    Stderr,
}

impl InspectorState {
    pub fn new(gpu_enabled: bool) -> Self {
        Self {
            job_id: None,
            details: None,
            gpu_enabled,
            sub_tab: SubTab::Overview,
            log_lines: Vec::new(),
            log_scroll: 0,
            log_mode: LogMode::Stdout,
            log_follow: true,
            log_last_len: 0,
            cpu_history: Vec::new(),
            mem_history: Vec::new(),
            gpu_history: Vec::new(),
        }
    }

    /// Extract a displayable string from a Slurm JSON field.
    ///
    /// Modern Slurm (22+) wraps many fields as objects like
    /// `{"number": 4, "set": true, "infinite": false}` or arrays like `["RUNNING"]`.
    /// This handles all these cases gracefully.
    fn get_str(&self, key: &str) -> String {
        let Some(val) = self.details.as_ref().and_then(|d| d.get(key)) else {
            return String::new();
        };
        slurm_val_to_string(val)
    }

    fn get_str_or(&self, key: &str, default: &str) -> String {
        let s = self.get_str(key);
        if s.is_empty() { default.to_string() } else { s }
    }

    /// Return a time field formatted as HH:MM:SS.
    /// Slurm encodes `time_limit` in minutes and `run_time` in seconds
    /// inside `{"number": N, ...}` objects, plain numbers, or strings.
    fn get_time_str(&self, key: &str, in_minutes: bool) -> String {
        let Some(val) = self.details.as_ref().and_then(|d| d.get(key)) else {
            return "N/A".to_string();
        };
        let secs = match val {
            serde_json::Value::Object(map) => {
                if map.get("infinite").and_then(|v| v.as_bool()).unwrap_or(false) {
                    return "UNLIMITED".to_string();
                }
                let n = map.get("number").and_then(|v| v.as_i64()).unwrap_or(0);
                if in_minutes { n * 60 } else { n }
            }
            serde_json::Value::Number(n) => {
                let n = n.as_i64().unwrap_or(0);
                if in_minutes { n * 60 } else { n }
            }
            serde_json::Value::String(s) => {
                // Already formatted time string like "01:30:00" — return as-is
                if s.contains(':') {
                    return s.clone();
                }
                // Numeric string
                match s.parse::<i64>() {
                    Ok(n) => if in_minutes { n * 60 } else { n },
                    Err(_) => return s.clone(),
                }
            }
            _ => return "N/A".to_string(),
        };
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        let s = secs % 60;
        format!("{h:02}:{m:02}:{s:02}")
    }

    /// Return a memory field formatted as MB or GB.
    /// Slurm encodes memory in MB inside `{"number": N, ...}` objects,
    /// plain numbers, or numeric strings.
    fn get_mem_str(&self, key: &str) -> String {
        let Some(val) = self.details.as_ref().and_then(|d| d.get(key)) else {
            return "N/A".to_string();
        };
        let mb = match val {
            serde_json::Value::Object(map) => {
                map.get("number").and_then(|v| v.as_i64()).unwrap_or(0)
            }
            serde_json::Value::Number(n) => n.as_i64().unwrap_or(0),
            serde_json::Value::String(s) => s.parse::<i64>().unwrap_or(0),
            _ => return "N/A".to_string(),
        };
        if mb == 0 {
            return "N/A".to_string();
        }
        if mb >= 1024 {
            format!("{:.1} GB", mb as f64 / 1024.0)
        } else {
            format!("{mb} MB")
        }
    }

    pub fn load_job(&mut self, job_id: &str, slurm: &dyn SlurmController) {
        self.job_id = Some(job_id.to_string());
        self.cpu_history.clear();
        self.mem_history.clear();
        self.gpu_history.clear();
        self.log_lines.clear();
        self.log_scroll = 0;
        self.log_mode = LogMode::Stdout;
        self.log_follow = true;
        self.log_last_len = 0;
        self.sub_tab = SubTab::Overview;
        self.refresh(slurm);
    }

    pub fn is_viewing_logs(&self) -> bool {
        self.sub_tab == SubTab::Logs
    }

    pub fn refresh(&mut self, slurm: &dyn SlurmController) {
        if let Some(ref jid) = self.job_id {
            self.details = slurm.get_job_details(jid);
            self.update_metrics(slurm);
            self.load_log_tail_inner(true); // force on full refresh
        }
    }

    fn update_metrics(&mut self, slurm: &dyn SlurmController) {
        let jid = match self.job_id {
            Some(ref j) => j.clone(),
            None => return,
        };

        let details = match self.details {
            Some(ref d) => d,
            None => return,
        };

        // Check for mock metrics (demo mode stores them in details)
        if let Some(metrics) = details.get("slurmterm_metrics") {
            if let Some(cpu_arr) = metrics.get("cpu").and_then(|v| v.as_array()) {
                self.cpu_history = cpu_arr.iter().filter_map(|v| v.as_f64()).collect();
            }
            if let Some(mem_arr) = metrics.get("mem").and_then(|v| v.as_array()) {
                self.mem_history = mem_arr.iter().filter_map(|v| v.as_f64()).collect();
            }
            if let Some(gpu_arr) = metrics.get("gpu").and_then(|v| v.as_array()) {
                self.gpu_history = gpu_arr.iter().filter_map(|v| v.as_f64()).collect();
            }
            return;
        }

        // Modern Slurm --json reports job_state as an array (["RUNNING"]);
        // job_state_of handles both that and the legacy plain string.
        if job_state_of(details) != "RUNNING" {
            return;
        }

        let sstat = match slurm.get_sstat(&jid) {
            Some(s) => s,
            None => return,
        };

        let total_mem_mb: i64 = details.get("minimum_memory_per_node")
            .and_then(|v| {
                if let Some(obj) = v.as_object() {
                    obj.get("number").and_then(|n| n.as_i64())
                } else {
                    v.as_i64()
                }
            })
            .unwrap_or(MEMORY_FALLBACK_MB as i64);

        let run_time: f64 = details.get("run_time")
            .and_then(|v| {
                if let Some(obj) = v.as_object() {
                    obj.get("number").and_then(|n| n.as_f64())
                } else {
                    v.as_f64()
                }
            })
            .unwrap_or(0.0);

        let cpu_val = parse_cpu_pct(&sstat.avg_cpu, run_time);
        let mem_val = parse_rss_to_pct(&sstat.max_rss, total_mem_mb);

        // GPU sampling is opt-in ([gpu] enabled in config): it may ssh to the
        // job's first node, which blocks the UI thread for up to one tick.
        let gpu_val = if self.gpu_enabled {
            crate::slurm_api::first_node_of(&self.get_str("nodes"))
                .map(|node| {
                    let samples = slurm.get_gpu_utilization(Some(&node));
                    if samples.is_empty() {
                        0.0
                    } else {
                        samples.iter().sum::<f64>() / samples.len() as f64
                    }
                })
                .unwrap_or(0.0)
        } else {
            0.0
        };

        self.cpu_history.push(cpu_val);
        self.mem_history.push(mem_val);
        self.gpu_history.push(gpu_val);

        for hist in [&mut self.cpu_history, &mut self.mem_history, &mut self.gpu_history] {
            if hist.len() > METRICS_ROLLING_WINDOW {
                let excess = hist.len() - METRICS_ROLLING_WINDOW;
                hist.drain(..excess);
            }
        }
    }

    pub fn load_log_tail(&mut self) {
        self.load_log_tail_inner(false);
    }

    /// Keep the scroll offset within the current log buffer. The buffer can
    /// shrink between polls (e.g. the job finished and its log path vanished),
    /// so every path that replaces `log_lines` must re-clamp.
    fn clamp_log_scroll(&mut self) {
        self.log_scroll = self.log_scroll.min(self.log_lines.len().saturating_sub(1));
    }

    /// Reload log file. If `force` is false, skip re-read when file size is unchanged.
    fn load_log_tail_inner(&mut self, force: bool) {
        let path_key = match self.log_mode {
            LogMode::Stdout => "standard_output",
            LogMode::Stderr => "standard_error",
        };
        let path = self.get_str(path_key);
        if path.is_empty() || path == "(null)" {
            self.log_lines = vec!["No log file path available".to_string()];
            self.clamp_log_scroll();
            return;
        }

        // Check file size first — skip read if unchanged (fast path for polling)
        if !force {
            if let Ok(meta) = std::fs::metadata(&path) {
                let len = meta.len();
                if len == self.log_last_len && !self.log_lines.is_empty() {
                    return; // file hasn't changed
                }
            }
        }

        match std::fs::File::open(&path) {
            Ok(file) => {
                if let Ok(meta) = file.metadata() {
                    self.log_last_len = meta.len();
                }
                let reader = std::io::BufReader::new(file);
                let all_lines: Vec<String> = reader.lines()
                    .map_while(|l| l.ok())
                    .map(|l| sanitize_log_line(&l))
                    .collect();
                let start = all_lines.len().saturating_sub(LOG_TAIL_LINES);
                self.log_lines = all_lines[start..].to_vec();
                if self.log_follow {
                    self.log_scroll = self.log_lines.len().saturating_sub(1);
                }
            }
            Err(e) => {
                self.log_lines = vec![format!("Cannot read {path}: {e}")];
            }
        }
        self.clamp_log_scroll();
    }

    pub fn handle_key(&mut self, key: KeyEvent, slurm: &dyn SlurmController) -> Action {
        match key.code {
            KeyCode::Esc => return Action::Back,
            KeyCode::Char('r') => {
                self.refresh(slurm);
                return Action::Refresh;
            }
            KeyCode::Char('s') => {
                if let Some(ref details) = self.details {
                    let form_state = crate::slurm_api::extract_form_state(details);
                    return Action::Resubmit(form_state);
                }
            }
            KeyCode::Char('e') => {
                self.log_mode = match self.log_mode {
                    LogMode::Stdout => LogMode::Stderr,
                    LogMode::Stderr => LogMode::Stdout,
                };
                self.log_last_len = 0; // force re-read on mode switch
                self.load_log_tail();
            }
            KeyCode::Char('f') => {
                self.log_follow = !self.log_follow;
                if self.log_follow {
                    self.log_scroll = self.log_lines.len().saturating_sub(1);
                }
            }
            KeyCode::Tab => {
                self.sub_tab = match self.sub_tab {
                    SubTab::Overview => SubTab::Logs,
                    SubTab::Logs => SubTab::Metrics,
                    SubTab::Metrics => SubTab::Overview,
                };
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.sub_tab == SubTab::Logs && self.log_scroll + 1 < self.log_lines.len() {
                    self.log_scroll += 1;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.sub_tab == SubTab::Logs {
                    self.log_scroll = self.log_scroll.saturating_sub(1);
                    self.log_follow = false;
                }
            }
            _ => {}
        }
        Action::None
    }

    pub fn draw(&self, f: &mut Frame, area: Rect) {
        if self.job_id.is_none() {
            let msg = Paragraph::new("Select a job from Monitor and press Enter to inspect it.")
                .style(Style::default().fg(theme::MUTED))
                .alignment(Alignment::Center);
            f.render_widget(msg, area);
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Length(1),
                Constraint::Min(0),
            ])
            .split(area);

        self.draw_header(f, chunks[0]);

        // Sub-tab bar
        let tab_spans: Vec<Span> = vec![
            Span::raw("  "),
            if self.sub_tab == SubTab::Overview {
                Span::styled("Overview", Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD))
            } else {
                Span::styled("Overview", Style::default().fg(theme::MUTED))
            },
            Span::styled("  |  ", Style::default().fg(theme::BORDER)),
            if self.sub_tab == SubTab::Logs {
                Span::styled("Logs", Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD))
            } else {
                Span::styled("Logs", Style::default().fg(theme::MUTED))
            },
            Span::styled("  |  ", Style::default().fg(theme::BORDER)),
            if self.sub_tab == SubTab::Metrics {
                Span::styled("Metrics", Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD))
            } else {
                Span::styled("Metrics", Style::default().fg(theme::MUTED))
            },
        ];
        f.render_widget(
            Paragraph::new(Line::from(tab_spans)).style(Style::default().bg(theme::SURFACE)),
            chunks[1],
        );

        match self.sub_tab {
            SubTab::Overview => self.draw_overview(f, chunks[2]),
            SubTab::Logs => self.draw_logs(f, chunks[2]),
            SubTab::Metrics => self.draw_metrics(f, chunks[2]),
        }
    }

    fn draw_header(&self, f: &mut Frame, area: Rect) {
        let name = self.get_str_or("name", "N/A");
        let state = self.get_str_or("job_state", "UNKNOWN");
        let jid = self.job_id.as_deref().unwrap_or("?");
        let state_color = crate::validators::state_color(&state);

        let header = Line::from(vec![
            Span::styled(
                format!("  {name} "),
                Style::default().fg(theme::TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {state} "),
                Style::default().fg(theme::BG).bg(state_color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  #{jid}"),
                Style::default().fg(theme::MUTED),
            ),
        ]);
        f.render_widget(Paragraph::new(header).style(Style::default().bg(theme::BG)), area);
    }

    fn draw_overview(&self, f: &mut Frame, area: Rect) {
        let jid = self.job_id.as_deref().unwrap_or("?").to_string();
        let fields: Vec<(&str, String)> = vec![
            ("Job ID", jid),
            ("Partition", self.get_str_or("partition", "N/A")),
            ("User", self.get_str_or("user_name", "N/A")),
            ("State", self.get_str_or("job_state", "N/A")),
            ("Work Dir", self.get_str_or("working_directory", "N/A")),
            ("Nodes", self.get_str_or("nodes", "N/A")),
            ("CPUs/Task", self.get_str_or("cpus_per_task", "N/A")),
            ("Memory", self.get_mem_str("minimum_memory_per_node")),
            ("Time Limit", self.get_time_str("time_limit", true)),
            ("Run Time", self.get_time_str("run_time", false)),
            ("stdout", self.get_str_or("standard_output", "N/A")),
            ("stderr", self.get_str_or("standard_error", "N/A")),
        ];

        let rows: Vec<Row> = fields
            .iter()
            .map(|(label, value)| {
                Row::new(vec![
                    Cell::from(format!("  {label}")).style(Style::default().fg(theme::ACCENT)),
                    Cell::from(value.as_str()).style(Style::default().fg(theme::TEXT)),
                ])
            })
            .collect();

        let table = Table::new(
            rows,
            [Constraint::Length(16), Constraint::Min(10)],
        )
        .block(Block::default().borders(Borders::NONE).style(Style::default().bg(theme::BG)));

        f.render_widget(table, area);
    }

    fn draw_logs(&self, f: &mut Frame, area: Rect) {
        let mode_label = match self.log_mode {
            LogMode::Stdout => "stdout",
            LogMode::Stderr => "stderr",
        };
        let path_key = match self.log_mode {
            LogMode::Stdout => "standard_output",
            LogMode::Stderr => "standard_error",
        };
        let path = self.get_str(path_key);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(area);

        let follow_indicator = if self.log_follow {
            Span::styled("  FOLLOW", Style::default().fg(theme::GREEN).add_modifier(Modifier::BOLD))
        } else {
            Span::styled("  PAUSED", Style::default().fg(theme::YELLOW))
        };
        let header = Line::from(vec![
            Span::styled(
                format!("  {mode_label}"),
                Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  {path}"),
                Style::default().fg(theme::DIM),
            ),
            follow_indicator,
        ]);
        f.render_widget(
            Paragraph::new(header).style(Style::default().bg(theme::SURFACE)),
            chunks[0],
        );

        let visible_height = chunks[1].height as usize;
        // log_scroll is clamped on reload, but this runs every frame between
        // state changes — re-derive a safe scroll so start can never pass end.
        let scroll = self.log_scroll.min(self.log_lines.len().saturating_sub(1));
        let start = scroll.saturating_sub(visible_height.saturating_sub(1));
        let end = (start + visible_height).min(self.log_lines.len());

        let text: Vec<Line> = self.log_lines[start..end]
            .iter()
            .map(|l| Line::from(l.as_str()))
            .collect();

        let log_block = Paragraph::new(text)
            .style(Style::default().fg(theme::TEXT).bg(theme::BG))
            .block(Block::default().borders(Borders::NONE));

        f.render_widget(log_block, chunks[1]);
    }

    fn draw_metrics(&self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
                Constraint::Ratio(1, 3),
            ])
            .split(area);

        self.draw_sparkline(f, chunks[0], "CPU %", &self.cpu_history, theme::ACCENT);
        self.draw_sparkline(f, chunks[1], "Memory %", &self.mem_history, theme::GREEN);
        self.draw_sparkline(f, chunks[2], "GPU %", &self.gpu_history, theme::PEACH);
    }

    fn draw_sparkline(&self, f: &mut Frame, area: Rect, title: &str, data: &[f64], color: Color) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme::BORDER))
            .title(Span::styled(format!(" {title} "), Style::default().fg(color).add_modifier(Modifier::BOLD)))
            .style(Style::default().bg(theme::BG));

        let inner = block.inner(area);
        f.render_widget(block, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        if data.is_empty() {
            let msg = Paragraph::new("No data yet")
                .style(Style::default().fg(theme::MUTED))
                .alignment(Alignment::Center);
            f.render_widget(msg, inner);
            return;
        }

        let vals: Vec<u64> = data.iter().map(|&v| v.clamp(0.0, 100.0) as u64).collect();
        let width = inner.width as usize;
        let start = vals.len().saturating_sub(width);
        let visible = &vals[start..];

        let sparkline = Sparkline::default()
            .data(visible)
            .max(100)
            .style(Style::default().fg(color));

        f.render_widget(sparkline, inner);

        if let Some(&last) = data.last() {
            let label = format!("{last:.0}%");
            let label_area = Rect::new(
                inner.x + inner.width.saturating_sub(label.len() as u16 + 1),
                inner.y,
                label.len() as u16 + 1,
                1,
            );
            f.render_widget(
                Paragraph::new(label).style(Style::default().fg(color).add_modifier(Modifier::BOLD)),
                label_area,
            );
        }
    }

    pub fn sub_tab_is_logs(&self) -> bool {
        self.sub_tab == SubTab::Logs
    }

    pub fn scroll_logs_down(&mut self) {
        if self.log_scroll + 1 < self.log_lines.len() {
            self.log_scroll += 1;
        }
    }

    pub fn scroll_logs_up(&mut self) {
        self.log_scroll = self.log_scroll.saturating_sub(1);
        self.log_follow = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn inspector_on_logs(log_lines: Vec<String>, log_scroll: usize) -> InspectorState {
        let mut insp = InspectorState::new(false);
        insp.job_id = Some("123".to_string());
        insp.sub_tab = SubTab::Logs;
        insp.log_lines = log_lines;
        insp.log_scroll = log_scroll;
        insp
    }

    fn draw_to_test_backend(insp: &InspectorState, width: u16, height: u16) {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| insp.draw(f, f.area())).unwrap();
    }

    /// Regression: the log buffer shrank (job finished, log path gone) while
    /// log_scroll still pointed past the end — draw must not panic.
    #[test]
    fn draw_logs_with_stale_scroll_does_not_panic() {
        let insp = inspector_on_logs(vec!["only line".to_string()], 500);
        draw_to_test_backend(&insp, 80, 24);
    }

    #[test]
    fn draw_logs_with_empty_buffer_does_not_panic() {
        let insp = inspector_on_logs(Vec::new(), 42);
        draw_to_test_backend(&insp, 80, 24);
    }

    #[test]
    fn draw_logs_in_tiny_terminal_does_not_panic() {
        let lines = (0..300).map(|i| format!("line {i}")).collect();
        let insp = inspector_on_logs(lines, 299);
        draw_to_test_backend(&insp, 10, 3);
    }

    #[test]
    fn clamp_log_scroll_pulls_offset_into_range() {
        let mut insp = inspector_on_logs(vec!["a".to_string()], 500);
        insp.clamp_log_scroll();
        assert_eq!(insp.log_scroll, 0);
        insp.log_lines.clear();
        insp.clamp_log_scroll();
        assert_eq!(insp.log_scroll, 0);
    }
}
