use std::collections::HashSet;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::slurm_api::{JobInfo, SlurmController, JOB_SIGNALS};
use crate::tabs::inspector::InspectorState;
use crate::theme;
use crate::validators::state_color;

pub enum Action {
    None,
    Refresh,
    /// Job IDs to cancel, plus a human description for the confirm dialog.
    CancelJobs(Vec<String>, String),
    HoldJobs(Vec<String>),
    ReleaseJobs(Vec<String>),
    RequeueJobs(Vec<String>, String),
    /// (job IDs, signal name, batch step only)
    SignalJobs(Vec<String>, String, bool),
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
    /// Passed to inspectors opened from this tab (config [gpu] enabled).
    pub gpu_enabled: bool,
    sort_col: SortCol,
    sort_asc: bool,
    /// Collapse the tasks of each job array into one row.
    pub group_arrays: bool,
    /// Open signal picker: (highlighted signal index, batch step only).
    signal_picker: Option<(usize, bool)>,
}

/// squeue's compact state codes (squeue(1), JOB STATE CODES).
fn state_code(state: &str) -> &str {
    match state {
        "PENDING" => "PD",
        "RUNNING" => "R",
        "SUSPENDED" => "S",
        "COMPLETING" => "CG",
        "COMPLETED" => "CD",
        "CONFIGURING" => "CF",
        "CANCELLED" => "CA",
        "FAILED" => "F",
        "TIMEOUT" => "TO",
        "PREEMPTED" => "PR",
        "NODE_FAIL" => "NF",
        "REQUEUED" => "RQ",
        "OUT_OF_MEMORY" => "OOM",
        "STOPPED" => "ST",
        other => other,
    }
}

/// Collapse array tasks (and the array's pending record) into one row per
/// array, keyed by the array master ID. Actions on that row target the
/// master ID, which Slurm applies to every task of the array.
pub fn group_array_jobs(jobs: &[JobInfo]) -> Vec<JobInfo> {
    let mut out: Vec<JobInfo> = Vec::new();
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut counts: Vec<Vec<(String, usize)>> = Vec::new();
    for j in jobs {
        if j.array_job_id.is_empty() {
            out.push(j.clone());
            counts.push(Vec::new());
            continue;
        }
        let i = *index.entry(j.array_job_id.clone()).or_insert_with(|| {
            let mut g = j.clone();
            g.job_id = j.array_job_id.clone();
            g.array_task_id.clear();
            g.array_task_string = "*".into();
            g.time_used = String::new();
            out.push(g);
            counts.push(Vec::new());
            out.len() - 1
        });
        let g = &mut out[i];
        // A pending record stands for several unstarted tasks; count it as
        // one entry since squeue does not report how many remain.
        let c = &mut counts[i];
        match c.iter_mut().find(|(st, _)| *st == j.state) {
            Some(e) => e.1 += 1,
            None => c.push((j.state.clone(), 1)),
        }
        if crate::validators::cmp_duration(&j.time_used, &g.time_used).is_gt() {
            g.time_used = j.time_used.clone();
        }
    }
    for (g, c) in out.iter_mut().zip(counts) {
        if c.is_empty() {
            continue;
        }
        // Row state: the most "active" one present
        for st in ["RUNNING", "COMPLETING", "PENDING"] {
            if c.iter().any(|(s, _)| s == st) {
                g.state = st.to_string();
                break;
            }
        }
        g.reason = c
            .iter()
            .map(|(s, n)| format!("{n}{}", state_code(s)))
            .collect::<Vec<_>>()
            .join(" ");
    }
    out
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
            gpu_enabled: false,
            sort_col: SortCol::Id,
            sort_asc: true,
            group_arrays: false,
            signal_picker: None,
        }
    }
}

impl MonitorState {
    pub fn poll(&mut self, slurm: &dyn SlurmController) {
        self.jobs = slurm.get_queue(None);
        // Prune stale selections
        // Array master IDs are valid targets too (grouped rows)
        let ids: HashSet<String> = self
            .jobs
            .iter()
            .flat_map(|j| [j.job_id.clone(), j.array_job_id.clone()])
            .collect();
        self.selected.retain(|id| ids.contains(id));
        // Refresh inline inspector if open
        if let Some(ref mut inspector) = self.inspector {
            inspector.refresh(slurm);
        }
    }

    fn filtered_jobs(&self) -> Vec<JobInfo> {
        let grouped;
        let source: &[JobInfo] = if self.group_arrays {
            grouped = group_array_jobs(&self.jobs);
            &grouped
        } else {
            &self.jobs
        };
        let mut result: Vec<JobInfo> = if self.search_query.is_empty() {
            source.to_vec()
        } else {
            let q = self.search_query.to_lowercase();
            source
                .iter()
                .filter(|j| {
                    j.name.to_lowercase().contains(&q)
                        || j.display_id().to_lowercase().contains(&q)
                        || j.state.to_lowercase().contains(&q)
                        || j.partition.to_lowercase().contains(&q)
                })
                .cloned()
                .collect()
        };
        let asc = self.sort_asc;
        result.sort_by(|a, b| {
            let ord = match self.sort_col {
                // squeue order: by array master, then task
                SortCol::Id => {
                    let key = |j: &JobInfo| {
                        if j.array_job_id.is_empty() { j.job_id.clone() } else { j.array_job_id.clone() }
                    };
                    // started tasks first, then the array's pending record
                    crate::validators::cmp_numeric(&key(a), &key(b))
                        .then_with(|| {
                            (!a.array_task_string.is_empty()).cmp(&!b.array_task_string.is_empty())
                        })
                        .then_with(|| crate::validators::cmp_numeric(&a.array_task_id, &b.array_task_id))
                }
                SortCol::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortCol::Partition => a.partition.cmp(&b.partition),
                SortCol::State => a.state.cmp(&b.state),
                SortCol::Time => crate::validators::cmp_duration(&a.time_used, &b.time_used),
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

    pub fn action_targets(&self) -> Vec<String> {
        if !self.selected.is_empty() {
            let mut v: Vec<String> = self.selected.iter().cloned().collect();
            v.sort_by(|a, b| crate::validators::cmp_numeric(a, b));
            v
        } else {
            self.get_cursor_job_id().into_iter().collect()
        }
    }

    /// Describe action targets for a confirm dialog, calling out whole
    /// arrays (an array master ID applies to every task).
    fn describe_targets(&self, targets: &[String]) -> String {
        let is_array = |id: &String| {
            self.jobs
                .iter()
                .any(|j| j.array_job_id == *id && (j.job_id != *id || !j.array_task_string.is_empty()))
        };
        match targets {
            [one] if self.group_arrays && is_array(one) => format!("entire job array {one}"),
            [one] => {
                let shown = self
                    .jobs
                    .iter()
                    .find(|j| j.job_id == *one)
                    .map(|j| j.display_id())
                    .unwrap_or_else(|| one.clone());
                format!("job {shown}")
            }
            many => format!("{} selected jobs", many.len()),
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

        if let Some((idx, batch)) = self.signal_picker {
            match key.code {
                KeyCode::Esc | KeyCode::Char('q') => self.signal_picker = None,
                KeyCode::Down | KeyCode::Char('j') => {
                    self.signal_picker = Some(((idx + 1).min(JOB_SIGNALS.len() - 1), batch));
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.signal_picker = Some((idx.saturating_sub(1), batch));
                }
                KeyCode::Char('b') => self.signal_picker = Some((idx, !batch)),
                KeyCode::Enter => {
                    self.signal_picker = None;
                    let targets = self.action_targets();
                    if !targets.is_empty() {
                        return Action::SignalJobs(targets, JOB_SIGNALS[idx].to_string(), batch);
                    }
                }
                _ => {}
            }
            return Action::None;
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
            KeyCode::Up | KeyCode::Char('k') => {
                let i = self.table_state.selected().unwrap_or(0);
                self.table_state.select(Some(i.saturating_sub(1)));
            }
            KeyCode::Enter | KeyCode::Char('i') => {
                if let Some(id) = self.get_cursor_job_id() {
                    let mut insp = InspectorState::new(self.gpu_enabled);
                    insp.load_job(&id, slurm);
                    self.inspector = Some(insp);
                }
            }
            KeyCode::Char('r') => {
                return Action::Refresh;
            }
            KeyCode::Char('x') => {
                let targets = self.action_targets();
                if !targets.is_empty() {
                    let what = self.describe_targets(&targets);
                    return Action::CancelJobs(targets, what);
                }
            }
            KeyCode::Char('R') => {
                let targets = self.action_targets();
                if !targets.is_empty() {
                    let what = self.describe_targets(&targets);
                    return Action::RequeueJobs(targets, what);
                }
            }
            KeyCode::Char('K') => {
                if !self.action_targets().is_empty() {
                    self.signal_picker = Some((0, false));
                }
            }
            KeyCode::Char('a') => {
                self.group_arrays = !self.group_arrays;
                self.table_state.select(Some(0));
                self.selected.clear();
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
                    Cell::from(format!("{mark}{}", j.display_id())).style(Style::default().fg(theme::TEXT)),
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
                Constraint::Length(16),
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

        if let Some((idx, batch)) = self.signal_picker {
            self.draw_signal_picker(f, area, idx, batch);
        }
    }

    fn draw_signal_picker(&self, f: &mut Frame, area: Rect, idx: usize, batch: bool) {
        let w = 34.min(area.width);
        let h = (JOB_SIGNALS.len() as u16 + 4).min(area.height);
        let popup = Rect::new(
            area.x + area.width.saturating_sub(w) / 2,
            area.y + area.height.saturating_sub(h) / 2,
            w,
            h,
        );
        let mut lines: Vec<Line> = JOB_SIGNALS
            .iter()
            .enumerate()
            .map(|(i, sig)| {
                let style = if i == idx {
                    Style::default().bg(theme::HIGHLIGHT).fg(theme::TEXT)
                } else {
                    Style::default().fg(theme::DIM)
                };
                Line::styled(format!("  SIG{sig}"), style)
            })
            .collect();
        lines.push(Line::styled(
            format!("  [b] batch step only: {}", if batch { "yes" } else { "no" }),
            Style::default().fg(theme::MUTED),
        ));
        f.render_widget(Clear, popup);
        f.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" scancel --signal ")
                    .border_style(Style::default().fg(theme::ACCENT))
                    .style(Style::default().bg(theme::SURFACE)),
            ),
            popup,
        );
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
            // Add the scroll offset so clicks map to the right row once the
            // table has scrolled past the top.
            let idx = self.table_state.offset() + (row - data_start) as usize;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock_slurm::MockSlurmController;
    use crossterm::event::KeyEvent;

    fn job(id: &str, state: &str, array: Option<(&str, Option<&str>, &str)>) -> JobInfo {
        let mut j = JobInfo {
            job_id: id.into(),
            name: "n".into(),
            state: state.into(),
            time_used: "00:01:00".into(),
            ..Default::default()
        };
        if let Some((a, t, pending)) = array {
            j.array_job_id = a.into();
            j.array_task_id = t.unwrap_or("").into();
            j.array_task_string = pending.into();
        }
        j
    }

    fn sample() -> Vec<JobInfo> {
        vec![
            job("10", "RUNNING", None),
            job("20", "PENDING", Some(("20", None, "2-9"))),
            job("21", "RUNNING", Some(("20", Some("0"), ""))),
            job("22", "RUNNING", Some(("20", Some("1"), ""))),
        ]
    }

    #[test]
    fn grouping_collapses_array_tasks() {
        let g = group_array_jobs(&sample());
        assert_eq!(g.len(), 2);
        let arr = g.iter().find(|j| j.job_id == "20").unwrap();
        assert_eq!(arr.display_id(), "20_[*]");
        assert_eq!(arr.state, "RUNNING");
        assert_eq!(arr.reason, "1PD 2R");
    }

    #[test]
    fn grouped_row_actions_target_the_array_id() {
        let slurm = MockSlurmController::new(0, Some(1));
        let mut m = MonitorState { jobs: sample(), ..Default::default() };
        m.handle_key(KeyEvent::from(KeyCode::Char('a')), &slurm);
        assert!(m.group_arrays);
        // Sorted by ID: row 0 = job 10, row 1 = array 20
        m.table_state.select(Some(1));
        match m.handle_key(KeyEvent::from(KeyCode::Char('x')), &slurm) {
            Action::CancelJobs(ids, what) => {
                assert_eq!(ids, vec!["20".to_string()]);
                assert_eq!(what, "entire job array 20");
            }
            _ => panic!("expected CancelJobs"),
        }
    }

    #[test]
    fn ungrouped_rows_use_squeue_ids_but_act_on_job_ids() {
        let slurm = MockSlurmController::new(0, Some(1));
        let mut m = MonitorState { jobs: sample(), ..Default::default() };
        let ids: Vec<String> = m.filtered_jobs().iter().map(|j| j.display_id()).collect();
        assert_eq!(ids, vec!["10", "20_0", "20_1", "20_[2-9]"]);
        m.table_state.select(Some(2));
        match m.handle_key(KeyEvent::from(KeyCode::Char('R')), &slurm) {
            Action::RequeueJobs(ids, what) => {
                assert_eq!(ids, vec!["22".to_string()]);
                assert_eq!(what, "job 20_1");
            }
            _ => panic!("expected RequeueJobs"),
        }
    }

    #[test]
    fn signal_picker_selects_signal_and_batch_flag() {
        let slurm = MockSlurmController::new(0, Some(1));
        let mut m = MonitorState { jobs: sample(), ..Default::default() };
        m.table_state.select(Some(0));
        m.handle_key(KeyEvent::from(KeyCode::Char('K')), &slurm);
        m.handle_key(KeyEvent::from(KeyCode::Char('j')), &slurm);
        m.handle_key(KeyEvent::from(KeyCode::Char('b')), &slurm);
        match m.handle_key(KeyEvent::from(KeyCode::Enter), &slurm) {
            Action::SignalJobs(ids, sig, batch) => {
                assert_eq!(ids, vec!["10".to_string()]);
                assert_eq!(sig, JOB_SIGNALS[1]);
                assert!(batch);
            }
            _ => panic!("expected SignalJobs"),
        }
        assert!(m.signal_picker.is_none());
    }

    #[test]
    fn search_matches_display_ids() {
        let mut m = MonitorState { jobs: sample(), ..Default::default() };
        m.search_query = "20_1".into();
        let f = m.filtered_jobs();
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].job_id, "22");
    }
}
