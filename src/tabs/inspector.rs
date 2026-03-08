use std::collections::HashMap;
use std::io::BufRead;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::slurm_api::SlurmController;
use crate::theme;
use crate::validators::{parse_rss_to_pct, parse_cpu_pct};

const METRICS_ROLLING_WINDOW: usize = 60;
const MEMORY_FALLBACK_MB: u64 = 64_000;
const LOG_TAIL_LINES: usize = 200;

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
    sub_tab: SubTab,
    // Logs
    log_lines: Vec<String>,
    log_scroll: usize,
    log_mode: LogMode,
    log_follow: bool,
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
    pub fn new() -> Self {
        Self {
            job_id: None,
            details: None,
            sub_tab: SubTab::Overview,
            log_lines: Vec::new(),
            log_scroll: 0,
            log_mode: LogMode::Stdout,
            log_follow: true,
            cpu_history: Vec::new(),
            mem_history: Vec::new(),
            gpu_history: Vec::new(),
        }
    }

    fn get_str(&self, key: &str) -> String {
        self.details
            .as_ref()
            .and_then(|d| d.get(key))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }

    fn get_str_or(&self, key: &str, default: &str) -> String {
        let s = self.get_str(key);
        if s.is_empty() { default.to_string() } else { s }
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
        self.sub_tab = SubTab::Overview;
        self.refresh(slurm);
    }

    pub fn refresh(&mut self, slurm: &dyn SlurmController) {
        if let Some(ref jid) = self.job_id {
            self.details = slurm.get_job_details(jid);
            self.update_metrics(slurm);
            self.load_log_tail();
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

        let state = details.get("job_state")
            .and_then(|v| v.as_str())
            .unwrap_or("UNKNOWN");
        if state != "RUNNING" {
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

        self.cpu_history.push(cpu_val);
        self.mem_history.push(mem_val);
        self.gpu_history.push(0.0);

        for hist in [&mut self.cpu_history, &mut self.mem_history, &mut self.gpu_history] {
            if hist.len() > METRICS_ROLLING_WINDOW {
                let excess = hist.len() - METRICS_ROLLING_WINDOW;
                hist.drain(..excess);
            }
        }
    }

    fn load_log_tail(&mut self) {
        let path_key = match self.log_mode {
            LogMode::Stdout => "standard_output",
            LogMode::Stderr => "standard_error",
        };
        let path = self.get_str(path_key);
        if path.is_empty() || path == "(null)" {
            self.log_lines = vec!["No log file path available".to_string()];
            return;
        }

        match std::fs::File::open(&path) {
            Ok(file) => {
                let reader = std::io::BufReader::new(file);
                let all_lines: Vec<String> = reader.lines()
                    .map_while(|l| l.ok())
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
            ("Work Dir", self.get_str_or("working_directory", "N/A")),
            ("Nodes", self.get_str_or("nodes", "N/A")),
            ("CPUs/Task", self.get_str_or("cpus_per_task", "N/A")),
            ("Memory", self.get_str_or("minimum_memory_per_node", "N/A")),
            ("Time Limit", self.get_str_or("time_limit", "N/A")),
            ("Run Time", self.get_str_or("run_time", "N/A")),
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
        let start = self.log_scroll.saturating_sub(visible_height.saturating_sub(1));
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
