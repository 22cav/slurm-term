use std::collections::HashSet;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::slurm_api::{JobInfo, SlurmController};
use crate::tabs::inspector::InspectorState;
use crate::theme;
use crate::validators::state_color;

pub enum Action {
    None,
    Refresh,
    CancelJobs(Vec<String>),
    HoldJobs(Vec<String>),
    ReleaseJobs(Vec<String>),
    Resubmit(std::collections::HashMap<String, String>),
}

#[derive(Clone, Copy, PartialEq)]
enum SortCol {
    Id,
    Name,
    Partition,
    State,
    Time,
}

impl SortCol {
    fn next(self) -> Self {
        match self {
            Self::Id => Self::Name,
            Self::Name => Self::Partition,
            Self::Partition => Self::State,
            Self::State => Self::Time,
            Self::Time => Self::Id,
        }
    }
}

pub struct MonitorState {
    pub jobs: Vec<JobInfo>,
    pub table_state: TableState,
    pub search_active: bool,
    pub search_query: String,
    pub selected: HashSet<String>,
    pub inspector: Option<InspectorState>,
    sort_col: SortCol,
    sort_asc: bool,
}

impl Default for MonitorState {
    fn default() -> Self {
        Self {
            jobs: Vec::new(),
            table_state: TableState::default(),
            search_active: false,
            search_query: String::new(),
            selected: HashSet::new(),
            inspector: None,
            sort_col: SortCol::Id,
            sort_asc: true,
        }
    }
}

impl MonitorState {
    pub fn poll(&mut self, slurm: &dyn SlurmController) {
        self.jobs = slurm.get_queue(None);
        // Prune stale selections
        let ids: HashSet<String> = self.jobs.iter().map(|j| j.job_id.clone()).collect();
        self.selected.retain(|id| ids.contains(id));
        // Refresh inline inspector if open
        if let Some(ref mut inspector) = self.inspector {
            inspector.refresh(slurm);
        }
    }

    fn filtered_jobs(&self) -> Vec<&JobInfo> {
        let mut result: Vec<&JobInfo> = if self.search_query.is_empty() {
            self.jobs.iter().collect()
        } else {
            let q = self.search_query.to_lowercase();
            self.jobs
                .iter()
                .filter(|j| {
                    j.name.to_lowercase().contains(&q)
                        || j.job_id.to_lowercase().contains(&q)
                        || j.state.to_lowercase().contains(&q)
                        || j.partition.to_lowercase().contains(&q)
                })
                .collect()
        };
        let asc = self.sort_asc;
        result.sort_by(|a, b| {
            let ord = match self.sort_col {
                SortCol::Id => a.job_id.cmp(&b.job_id),
                SortCol::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortCol::Partition => a.partition.cmp(&b.partition),
                SortCol::State => a.state.cmp(&b.state),
                SortCol::Time => a.time_used.cmp(&b.time_used),
            };
            if asc { ord } else { ord.reverse() }
        });
        result
    }

    fn get_cursor_job_id(&self) -> Option<String> {
        let filtered = self.filtered_jobs();
        self.table_state
            .selected()
            .and_then(|i| filtered.get(i))
            .map(|j| j.job_id.clone())
    }

    fn action_targets(&self) -> Vec<String> {
        if !self.selected.is_empty() {
            self.selected.iter().cloned().collect()
        } else {
            self.get_cursor_job_id().into_iter().collect()
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent, slurm: &dyn SlurmController) -> Action {
        // If inspector is open, delegate keys to it
        if let Some(ref mut inspector) = self.inspector {
            use crate::tabs::inspector;
            match key.code {
                KeyCode::Esc => {
                    self.inspector = None;
                    return Action::None;
                }
                _ => {
                    let action = inspector.handle_key(key, slurm);
                    match action {
                        inspector::Action::Back => {
                            self.inspector = None;
                        }
                        inspector::Action::Refresh => {}
                        inspector::Action::Resubmit(form_state) => {
                            return Action::Resubmit(form_state);
                        }
                        inspector::Action::None => {}
                    }
                    return Action::None;
                }
            }
        }

        if self.search_active {
            match key.code {
                KeyCode::Esc => {
                    self.search_active = false;
                    self.search_query.clear();
                }
                KeyCode::Enter => {
                    self.search_active = false;
                }
                KeyCode::Backspace => {
                    self.search_query.pop();
                }
                KeyCode::Char(c) => {
                    self.search_query.push(c);
                }
                _ => {}
            }
            return Action::None;
        }

        let filtered_len = self.filtered_jobs().len();

        match key.code {
            KeyCode::Char('/') => {
                self.search_active = true;
                return Action::None;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let i = self.table_state.selected().unwrap_or(0);
                if filtered_len > 0 {
                    self.table_state.select(Some((i + 1).min(filtered_len - 1)));
                }
            }
            KeyCode::Up => {
                let i = self.table_state.selected().unwrap_or(0);
                self.table_state.select(Some(i.saturating_sub(1)));
            }
            KeyCode::Enter | KeyCode::Char('i') => {
                if let Some(id) = self.get_cursor_job_id() {
                    let mut insp = InspectorState::new();
                    insp.load_job(&id, slurm);
                    self.inspector = Some(insp);
                }
            }
            KeyCode::Char('r') => {
                return Action::Refresh;
            }
            KeyCode::Char('k') => {
                let targets = self.action_targets();
                if !targets.is_empty() {
                    return Action::CancelJobs(targets);
                }
            }
            KeyCode::Char('h') => {
                let targets = self.action_targets();
                if !targets.is_empty() {
                    return Action::HoldJobs(targets);
                }
            }
            KeyCode::Char('u') => {
                let targets = self.action_targets();
                if !targets.is_empty() {
                    return Action::ReleaseJobs(targets);
                }
            }
            KeyCode::Char(' ') => {
                if let Some(id) = self.get_cursor_job_id() {
                    if self.selected.contains(&id) {
                        self.selected.remove(&id);
                    } else {
                        self.selected.insert(id);
                    }
                }
            }
            KeyCode::Char('s') => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.sort_asc = !self.sort_asc;
                } else {
                    self.sort_col = self.sort_col.next();
                }
            }
            KeyCode::Esc => {
                if !self.selected.is_empty() {
                    self.selected.clear();
                } else if !self.search_query.is_empty() {
                    self.search_query.clear();
                }
            }
            _ => {}
        }
        Action::None
    }

    pub fn draw(&mut self, f: &mut Frame, area: Rect) {
        // Split for inline inspector
        let (table_area, inspector_area) = if self.inspector.is_some() {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
                .split(area);
            (chunks[0], Some(chunks[1]))
        } else {
            (area, None)
        };

        self.draw_table(f, table_area);

        if let Some(ref inspector) = self.inspector {
            if let Some(insp_area) = inspector_area {
                // Draw a separator line and inspector content
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(0), Constraint::Min(0)])
                    .split(insp_area);
                inspector.draw(f, chunks[1]);
            }
        }
    }

    fn draw_table(&mut self, f: &mut Frame, area: Rect) {
        let filtered = self.filtered_jobs();

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(if self.search_active || !self.search_query.is_empty() {
                vec![Constraint::Length(1), Constraint::Min(0)]
            } else {
                vec![Constraint::Length(0), Constraint::Min(0)]
            })
            .split(area);

        // Search bar
        if self.search_active || !self.search_query.is_empty() {
            let search_style = if self.search_active {
                Style::default().fg(theme::ACCENT).bg(theme::SURFACE)
            } else {
                Style::default().fg(theme::MUTED).bg(theme::BG)
            };
            let cursor = if self.search_active { "▏" } else { "" };
            f.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled("  / ", search_style),
                    Span::styled(format!("{}{cursor}", self.search_query), search_style),
                ])).style(search_style),
                chunks[0],
            );
        }

        // Empty state
        if filtered.is_empty() {
            let msg = if !self.search_query.is_empty() {
                "No jobs match your search"
            } else if self.jobs.is_empty() {
                "No active jobs"
            } else {
                "No jobs to display"
            };
            f.render_widget(
                Paragraph::new(msg)
                    .style(Style::default().fg(theme::MUTED))
                    .alignment(Alignment::Center),
                Rect::new(chunks[1].x, chunks[1].y + chunks[1].height / 3, chunks[1].width, 1),
            );
            return;
        }

        // Table
        let dir = if self.sort_asc { "+" } else { "-" };
        let h = |label: &str, col: SortCol| -> String {
            if self.sort_col == col {
                format!("{label}{dir}")
            } else {
                label.to_string()
            }
        };
        let header = Row::new(vec![
            h("  ID", SortCol::Id),
            h("Name", SortCol::Name),
            h("Partition", SortCol::Partition),
            h("State", SortCol::State),
            h("Time", SortCol::Time),
            "N".to_string(),
            "Reason".to_string(),
        ])
            .style(Style::default().add_modifier(Modifier::BOLD).fg(theme::ACCENT))
            .bottom_margin(0);

        let rows: Vec<Row> = filtered
            .iter()
            .map(|j| {
                let mark = if self.selected.contains(&j.job_id) {
                    "* "
                } else {
                    "  "
                };
                let color = state_color(&j.state);
                Row::new(vec![
                    Cell::from(format!("{mark}{}", j.job_id)).style(Style::default().fg(theme::TEXT)),
                    Cell::from(j.name.clone()).style(Style::default().fg(theme::TEXT)),
                    Cell::from(j.partition.clone()).style(Style::default().fg(theme::DIM)),
                    Cell::from(j.state.clone()).style(Style::default().fg(color)),
                    Cell::from(j.time_used.clone()).style(Style::default().fg(theme::DIM)),
                    Cell::from(j.nodes.clone()).style(Style::default().fg(theme::DIM)),
                    Cell::from(j.reason.clone()).style(Style::default().fg(theme::MUTED)),
                ])
            })
            .collect();

        let table = Table::new(
            rows,
            [
                Constraint::Length(10),
                Constraint::Min(16),
                Constraint::Length(10),
                Constraint::Length(14),
                Constraint::Length(10),
                Constraint::Length(3),
                Constraint::Min(8),
            ],
        )
        .header(header)
        .block(Block::default().borders(Borders::NONE).style(Style::default().bg(theme::BG)))
        .row_highlight_style(Style::default().bg(theme::HIGHLIGHT).fg(theme::TEXT));

        f.render_stateful_widget(table, chunks[1], &mut self.table_state);
    }

    pub fn handle_mouse_click(&mut self, row: u16, _col: u16, _area: &Rect) {
        if self.inspector.is_some() {
            return; // Don't handle table clicks when inspector is open
        }
        // Account for search bar
        let header_offset: u16 = if self.search_active || !self.search_query.is_empty() { 1 } else { 0 };
        // Table header row
        let table_header: u16 = 1;
        let data_start = header_offset + table_header;

        if row >= data_start {
            let idx = (row - data_start) as usize;
            let filtered_len = self.filtered_jobs().len();
            if idx < filtered_len {
                self.table_state.select(Some(idx));
            }
        }
    }

    pub fn scroll_down(&mut self) {
        if let Some(ref mut inspector) = self.inspector {
            if inspector.sub_tab_is_logs() {
                inspector.scroll_logs_down();
            }
            return;
        }
        let filtered_len = self.filtered_jobs().len();
        if filtered_len > 0 {
            let i = self.table_state.selected().unwrap_or(0);
            self.table_state.select(Some((i + 1).min(filtered_len - 1)));
        }
    }

    pub fn scroll_up(&mut self) {
        if let Some(ref mut inspector) = self.inspector {
            if inspector.sub_tab_is_logs() {
                inspector.scroll_logs_up();
            }
            return;
        }
        let i = self.table_state.selected().unwrap_or(0);
        self.table_state.select(Some(i.saturating_sub(1)));
    }
}
