use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::slurm_api::{SacctRow, SlurmController};
use crate::theme;

pub enum Action {
    None,
    Inspect(String), // job_id
    Refresh,
}

const TIME_WINDOWS: &[(u32, &str)] = &[
    (1, "1 day"),
    (3, "3 days"),
    (7, "7 days"),
    (14, "14 days"),
    (30, "30 days"),
];

#[derive(Clone, Copy, PartialEq)]
enum SortCol {
    JobId,
    Name,
    Partition,
    State,
    Elapsed,
    Exit,
}

impl SortCol {
    fn next(self) -> Self {
        match self {
            Self::JobId => Self::Name,
            Self::Name => Self::Partition,
            Self::Partition => Self::State,
            Self::State => Self::Elapsed,
            Self::Elapsed => Self::Exit,
            Self::Exit => Self::JobId,
        }
    }
}

pub struct HistoryState {
    pub rows: Vec<SacctRow>,
    pub since_days: u32,
    time_idx: usize,
    table_state: TableState,
    sort_col: SortCol,
    sort_asc: bool,
}

impl HistoryState {
    pub fn new(since_days: u32) -> Self {
        let time_idx = TIME_WINDOWS
            .iter()
            .position(|&(d, _)| d == since_days)
            .unwrap_or(0);
        Self {
            rows: Vec::new(),
            since_days,
            time_idx,
            table_state: TableState::default(),
            sort_col: SortCol::JobId,
            sort_asc: true,
        }
    }

    pub fn poll(&mut self, slurm: &dyn SlurmController) {
        let start = format!("now-{}days", self.since_days);
        self.rows = slurm.get_sacct(None, Some(&start));
    }

    fn sorted_rows(&self) -> Vec<&SacctRow> {
        let mut sorted: Vec<&SacctRow> = self.rows.iter().collect();
        let asc = self.sort_asc;
        sorted.sort_by(|a, b| {
            let ord = match self.sort_col {
                SortCol::JobId => a.job_id.cmp(&b.job_id),
                SortCol::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortCol::Partition => a.partition.cmp(&b.partition),
                SortCol::State => a.state.cmp(&b.state),
                SortCol::Elapsed => a.elapsed.cmp(&b.elapsed),
                SortCol::Exit => a.exit_code.cmp(&b.exit_code),
            };
            if asc { ord } else { ord.reverse() }
        });
        sorted
    }

    pub fn handle_key(&mut self, key: KeyEvent, _slurm: &dyn SlurmController) -> Action {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                let len = self.rows.len();
                if len > 0 {
                    let i = self.table_state.selected().map_or(0, |i| (i + 1).min(len - 1));
                    self.table_state.select(Some(i));
                }
                Action::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.table_state.selected().map_or(0, |i| i.saturating_sub(1));
                self.table_state.select(Some(i));
                Action::None
            }
            KeyCode::Enter => {
                if let Some(i) = self.table_state.selected() {
                    let sorted = self.sorted_rows();
                    if let Some(row) = sorted.get(i) {
                        return Action::Inspect(row.job_id.clone());
                    }
                }
                Action::None
            }
            KeyCode::Left => {
                if self.time_idx > 0 {
                    self.time_idx -= 1;
                    self.since_days = TIME_WINDOWS[self.time_idx].0;
                    return Action::Refresh;
                }
                Action::None
            }
            KeyCode::Right => {
                if self.time_idx + 1 < TIME_WINDOWS.len() {
                    self.time_idx += 1;
                    self.since_days = TIME_WINDOWS[self.time_idx].0;
                    return Action::Refresh;
                }
                Action::None
            }
            KeyCode::Char('r') => Action::Refresh,
            KeyCode::Char('s') => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.sort_asc = !self.sort_asc;
                } else {
                    self.sort_col = self.sort_col.next();
                }
                Action::None
            }
            _ => Action::None,
        }
    }

    pub fn draw(&mut self, f: &mut Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(area);

        // Time window selector
        let window_text = TIME_WINDOWS
            .iter()
            .enumerate()
            .map(|(i, &(_, label))| {
                if i == self.time_idx {
                    Span::styled(
                        format!(" [{label}] "),
                        Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD),
                    )
                } else {
                    Span::styled(format!("  {label}  "), Style::default().fg(theme::MUTED))
                }
            })
            .collect::<Vec<_>>();
        let mut spans = vec![Span::styled(
            "  Window:",
            Style::default().fg(theme::DIM),
        )];
        spans.extend(window_text);
        spans.push(Span::styled(
            format!("  {} jobs", self.rows.len()),
            Style::default().fg(theme::MUTED),
        ));
        f.render_widget(
            Paragraph::new(Line::from(spans)).style(Style::default().bg(theme::SURFACE)),
            chunks[0],
        );

        // Table
        let dir = if self.sort_asc { "+" } else { "-" };
        let h = |label: &str, col: SortCol| -> String {
            if self.sort_col == col { format!("{label}{dir}") } else { label.to_string() }
        };
        let header = Row::new(vec![
            Cell::from(h("  JobID", SortCol::JobId)),
            Cell::from(h("Name", SortCol::Name)),
            Cell::from(h("Partition", SortCol::Partition)),
            Cell::from(h("State", SortCol::State)),
            Cell::from(h("Elapsed", SortCol::Elapsed)),
            Cell::from("TotalCPU"),
            Cell::from("MaxRSS"),
            Cell::from(h("Exit", SortCol::Exit)),
        ])
        .style(Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD));

        let mut indices: Vec<usize> = (0..self.rows.len()).collect();
        let asc = self.sort_asc;
        indices.sort_by(|&a, &b| {
            let ra = &self.rows[a];
            let rb = &self.rows[b];
            let ord = match self.sort_col {
                SortCol::JobId => ra.job_id.cmp(&rb.job_id),
                SortCol::Name => ra.name.to_lowercase().cmp(&rb.name.to_lowercase()),
                SortCol::Partition => ra.partition.cmp(&rb.partition),
                SortCol::State => ra.state.cmp(&rb.state),
                SortCol::Elapsed => ra.elapsed.cmp(&rb.elapsed),
                SortCol::Exit => ra.exit_code.cmp(&rb.exit_code),
            };
            if asc { ord } else { ord.reverse() }
        });

        let rows: Vec<Row> = indices
            .iter()
            .map(|&i| {
                let r = &self.rows[i];
                let state_style = match r.state.as_str() {
                    "COMPLETED" => Style::default().fg(theme::GREEN),
                    "FAILED" | "NODE_FAIL" => Style::default().fg(theme::RED),
                    "TIMEOUT" | "CANCELLED" => Style::default().fg(theme::YELLOW),
                    "RUNNING" => Style::default().fg(theme::ACCENT),
                    "PENDING" => Style::default().fg(theme::MUTED),
                    _ => Style::default().fg(theme::DIM),
                };
                Row::new(vec![
                    Cell::from(format!("  {}", r.job_id)).style(Style::default().fg(theme::TEXT)),
                    Cell::from(r.name.as_str()).style(Style::default().fg(theme::TEXT)),
                    Cell::from(r.partition.as_str()).style(Style::default().fg(theme::DIM)),
                    Cell::from(r.state.as_str()).style(state_style),
                    Cell::from(r.elapsed.as_str()).style(Style::default().fg(theme::DIM)),
                    Cell::from(r.total_cpu.as_str()).style(Style::default().fg(theme::DIM)),
                    Cell::from(r.max_rss.as_str()).style(Style::default().fg(theme::DIM)),
                    Cell::from(r.exit_code.as_str()).style(Style::default().fg(theme::MUTED)),
                ])
            })
            .collect();

        let widths = [
            Constraint::Length(12),
            Constraint::Min(12),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(10),
            Constraint::Length(6),
        ];

        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::default().borders(Borders::NONE).style(Style::default().bg(theme::BG)))
            .row_highlight_style(Style::default().bg(theme::HIGHLIGHT).fg(theme::TEXT));

        f.render_stateful_widget(table, chunks[1], &mut self.table_state);
    }

    pub fn handle_mouse_click(&mut self, row: u16, _col: u16, _area: &Rect) {
        // Row 0 = time window selector, row 1 = table header, data starts at row 2
        if row >= 2 {
            let idx = (row - 2) as usize;
            if idx < self.rows.len() {
                self.table_state.select(Some(idx));
            }
        }
    }

    pub fn scroll_down(&mut self) {
        let len = self.rows.len();
        if len > 0 {
            let i = self.table_state.selected().map_or(0, |i| (i + 1).min(len - 1));
            self.table_state.select(Some(i));
        }
    }

    pub fn scroll_up(&mut self) {
        let i = self.table_state.selected().map_or(0, |i| i.saturating_sub(1));
        self.table_state.select(Some(i));
    }
}
