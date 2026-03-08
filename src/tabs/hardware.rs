use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::slurm_api::{SinfoRow, NodeInfoRow, StorageInfo, SlurmController};
use crate::theme;

pub enum Action {
    None,
    Refresh,
}

#[derive(Clone, Copy, PartialEq)]
enum SubTab {
    Partitions,
    Nodes,
    Storage,
}

#[derive(Clone, Copy, PartialEq)]
enum PartSortCol {
    Partition,
    Avail,
    Nodes,
    State,
    Cpus,
}

impl PartSortCol {
    fn next(self) -> Self {
        match self {
            Self::Partition => Self::Avail,
            Self::Avail => Self::Nodes,
            Self::Nodes => Self::State,
            Self::State => Self::Cpus,
            Self::Cpus => Self::Partition,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum NodeSortCol {
    Node,
    State,
    Cpus,
    Mem,
    Load,
}

impl NodeSortCol {
    fn next(self) -> Self {
        match self {
            Self::Node => Self::State,
            Self::State => Self::Cpus,
            Self::Cpus => Self::Mem,
            Self::Mem => Self::Load,
            Self::Load => Self::Node,
        }
    }
}

pub struct HardwareState {
    pub partitions: Vec<SinfoRow>,
    pub nodes: Vec<NodeInfoRow>,
    pub storage: Vec<StorageInfo>,
    sub_tab: SubTab,
    part_state: TableState,
    node_state: TableState,
    storage_state: TableState,
    part_sort_col: PartSortCol,
    part_sort_asc: bool,
    node_sort_col: NodeSortCol,
    node_sort_asc: bool,
}

impl HardwareState {
    pub fn new() -> Self {
        Self {
            partitions: Vec::new(),
            nodes: Vec::new(),
            storage: Vec::new(),
            sub_tab: SubTab::Partitions,
            part_state: TableState::default(),
            node_state: TableState::default(),
            storage_state: TableState::default(),
            part_sort_col: PartSortCol::Partition,
            part_sort_asc: true,
            node_sort_col: NodeSortCol::Node,
            node_sort_asc: true,
        }
    }

    pub fn poll(&mut self, slurm: &dyn SlurmController) {
        self.partitions = slurm.get_sinfo();
        self.nodes = slurm.get_node_info();
        self.storage = slurm.get_storage();
    }

    pub fn handle_key(&mut self, key: KeyEvent, _slurm: &dyn SlurmController) -> Action {
        match key.code {
            KeyCode::Tab => {
                self.sub_tab = match self.sub_tab {
                    SubTab::Partitions => SubTab::Nodes,
                    SubTab::Nodes => SubTab::Storage,
                    SubTab::Storage => SubTab::Partitions,
                };
                Action::None
            }
            KeyCode::Char('r') => Action::Refresh,
            KeyCode::Char('s') => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    match self.sub_tab {
                        SubTab::Partitions => self.part_sort_asc = !self.part_sort_asc,
                        SubTab::Nodes => self.node_sort_asc = !self.node_sort_asc,
                        SubTab::Storage => {}
                    }
                } else {
                    match self.sub_tab {
                        SubTab::Partitions => self.part_sort_col = self.part_sort_col.next(),
                        SubTab::Nodes => self.node_sort_col = self.node_sort_col.next(),
                        SubTab::Storage => {}
                    }
                }
                Action::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                match self.sub_tab {
                    SubTab::Partitions => {
                        let len = self.partitions.len();
                        if len > 0 {
                            let i = self.part_state.selected().map_or(0, |i| (i + 1).min(len - 1));
                            self.part_state.select(Some(i));
                        }
                    }
                    SubTab::Nodes => {
                        let len = self.nodes.len();
                        if len > 0 {
                            let i = self.node_state.selected().map_or(0, |i| (i + 1).min(len - 1));
                            self.node_state.select(Some(i));
                        }
                    }
                    SubTab::Storage => {
                        let len = self.storage.len();
                        if len > 0 {
                            let i = self.storage_state.selected().map_or(0, |i| (i + 1).min(len - 1));
                            self.storage_state.select(Some(i));
                        }
                    }
                }
                Action::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                match self.sub_tab {
                    SubTab::Partitions => {
                        let i = self.part_state.selected().map_or(0, |i| i.saturating_sub(1));
                        self.part_state.select(Some(i));
                    }
                    SubTab::Nodes => {
                        let i = self.node_state.selected().map_or(0, |i| i.saturating_sub(1));
                        self.node_state.select(Some(i));
                    }
                    SubTab::Storage => {
                        let i = self.storage_state.selected().map_or(0, |i| i.saturating_sub(1));
                        self.storage_state.select(Some(i));
                    }
                }
                Action::None
            }
            _ => Action::None,
        }
    }

    pub fn draw(&mut self, f: &mut Frame, area: Rect) {
        // Header with sub-tab selector
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .split(area);

        let tab_spans: Vec<Span> = vec![
            Span::raw("  "),
            if self.sub_tab == SubTab::Partitions {
                Span::styled("Partitions", Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD))
            } else {
                Span::styled("Partitions", Style::default().fg(theme::MUTED))
            },
            Span::styled("  |  ", Style::default().fg(theme::BORDER)),
            if self.sub_tab == SubTab::Nodes {
                Span::styled("Nodes", Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD))
            } else {
                Span::styled("Nodes", Style::default().fg(theme::MUTED))
            },
            Span::styled("  |  ", Style::default().fg(theme::BORDER)),
            if self.sub_tab == SubTab::Storage {
                Span::styled("Storage", Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD))
            } else {
                Span::styled("Storage", Style::default().fg(theme::MUTED))
            },
        ];
        f.render_widget(
            Paragraph::new(Line::from(tab_spans)).style(Style::default().bg(theme::SURFACE)),
            chunks[0],
        );

        match self.sub_tab {
            SubTab::Partitions => self.draw_partitions(f, chunks[1]),
            SubTab::Nodes => self.draw_nodes(f, chunks[1]),
            SubTab::Storage => self.draw_storage(f, chunks[1]),
        }
    }

    fn draw_partitions(&mut self, f: &mut Frame, area: Rect) {
        let dir = if self.part_sort_asc { "+" } else { "-" };
        let h = |label: &str, col: PartSortCol| -> String {
            if self.part_sort_col == col { format!("{label}{dir}") } else { label.to_string() }
        };
        let header = Row::new(vec![
            Cell::from(h("  Partition", PartSortCol::Partition)),
            Cell::from(h("Avail", PartSortCol::Avail)),
            Cell::from("TimeLimit"),
            Cell::from(h("Nodes", PartSortCol::Nodes)),
            Cell::from(h("State", PartSortCol::State)),
            Cell::from(h("CPUs", PartSortCol::Cpus)),
            Cell::from("Mem(GB)"),
            Cell::from("GRES"),
        ])
        .style(Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD));

        let mut sorted: Vec<&SinfoRow> = self.partitions.iter().collect();
        let asc = self.part_sort_asc;
        sorted.sort_by(|a, b| {
            let ord = match self.part_sort_col {
                PartSortCol::Partition => a.partition.cmp(&b.partition),
                PartSortCol::Avail => a.avail.cmp(&b.avail),
                PartSortCol::Nodes => a.nodes.cmp(&b.nodes),
                PartSortCol::State => a.state.cmp(&b.state),
                PartSortCol::Cpus => a.cpus.cmp(&b.cpus),
            };
            if asc { ord } else { ord.reverse() }
        });

        let rows: Vec<Row> = sorted
            .iter()
            .map(|p| {
                let state_style = match p.state.as_str() {
                    "up" => Style::default().fg(theme::GREEN),
                    "down" => Style::default().fg(theme::RED),
                    _ => Style::default().fg(theme::YELLOW),
                };
                Row::new(vec![
                    Cell::from(format!("  {}", p.partition)).style(Style::default().fg(theme::TEXT)),
                    Cell::from(p.avail.as_str()).style(state_style),
                    Cell::from(p.timelimit.as_str()).style(Style::default().fg(theme::DIM)),
                    Cell::from(p.nodes.as_str()).style(Style::default().fg(theme::DIM)),
                    Cell::from(p.state.as_str()).style(state_style),
                    Cell::from(p.cpus.as_str()).style(Style::default().fg(theme::DIM)),
                    Cell::from(p.memory.as_str()).style(Style::default().fg(theme::DIM)),
                    Cell::from(p.gres.as_str()).style(Style::default().fg(theme::MUTED)),
                ])
            })
            .collect();

        let widths = [
            Constraint::Min(14),
            Constraint::Length(6),
            Constraint::Length(12),
            Constraint::Length(6),
            Constraint::Length(8),
            Constraint::Length(6),
            Constraint::Length(8),
            Constraint::Min(10),
        ];

        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::default().borders(Borders::NONE).style(Style::default().bg(theme::BG)))
            .row_highlight_style(Style::default().bg(theme::HIGHLIGHT).fg(theme::TEXT));

        f.render_stateful_widget(table, area, &mut self.part_state);
    }

    fn draw_nodes(&mut self, f: &mut Frame, area: Rect) {
        let dir = if self.node_sort_asc { "+" } else { "-" };
        let h = |label: &str, col: NodeSortCol| -> String {
            if self.node_sort_col == col { format!("{label}{dir}") } else { label.to_string() }
        };
        let header = Row::new(vec![
            Cell::from(h("  Node", NodeSortCol::Node)),
            Cell::from(h("State", NodeSortCol::State)),
            Cell::from(h("CPUs", NodeSortCol::Cpus)),
            Cell::from(h("Mem(GB)", NodeSortCol::Mem)),
            Cell::from("GRES"),
            Cell::from("Partitions"),
            Cell::from(h("Load", NodeSortCol::Load)),
            Cell::from("Free(GB)"),
        ])
        .style(Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD));

        let mut sorted: Vec<&NodeInfoRow> = self.nodes.iter().collect();
        let asc = self.node_sort_asc;
        sorted.sort_by(|a, b| {
            let get = |n: &NodeInfoRow, k: &str| n.fields.get(k).cloned().unwrap_or_default();
            let ord = match self.node_sort_col {
                NodeSortCol::Node => get(a, "NodeName").cmp(&get(b, "NodeName")),
                NodeSortCol::State => get(a, "State").cmp(&get(b, "State")),
                NodeSortCol::Cpus => get(a, "CPUTot").cmp(&get(b, "CPUTot")),
                NodeSortCol::Mem => get(a, "RealMemory").cmp(&get(b, "RealMemory")),
                NodeSortCol::Load => get(a, "CPULoad").cmp(&get(b, "CPULoad")),
            };
            if asc { ord } else { ord.reverse() }
        });

        let rows: Vec<Row> = sorted
            .iter()
            .map(|n| {
                let f = |key: &str| -> String {
                    n.fields.get(key).cloned().unwrap_or_default()
                };
                let state = f("State");
                let state_style = if state.contains("idle") {
                    Style::default().fg(theme::GREEN)
                } else if state.contains("alloc") {
                    Style::default().fg(theme::YELLOW)
                } else if state.contains("down") || state.contains("drain") {
                    Style::default().fg(theme::RED)
                } else {
                    Style::default().fg(theme::DIM)
                };
                Row::new(vec![
                    Cell::from(format!("  {}", f("NodeName"))).style(Style::default().fg(theme::TEXT)),
                    Cell::from(state).style(state_style),
                    Cell::from(f("CPUTot")).style(Style::default().fg(theme::DIM)),
                    Cell::from(f("RealMemory")).style(Style::default().fg(theme::DIM)),
                    Cell::from(f("Gres")).style(Style::default().fg(theme::MUTED)),
                    Cell::from(f("Partitions")).style(Style::default().fg(theme::MUTED)),
                    Cell::from(f("CPULoad")).style(Style::default().fg(theme::DIM)),
                    Cell::from(f("FreeMem")).style(Style::default().fg(theme::DIM)),
                ])
            })
            .collect();

        let widths = [
            Constraint::Min(14),
            Constraint::Length(10),
            Constraint::Length(6),
            Constraint::Length(8),
            Constraint::Min(10),
            Constraint::Min(12),
            Constraint::Length(8),
            Constraint::Length(8),
        ];

        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::default().borders(Borders::NONE).style(Style::default().bg(theme::BG)))
            .row_highlight_style(Style::default().bg(theme::HIGHLIGHT).fg(theme::TEXT));

        f.render_stateful_widget(table, area, &mut self.node_state);
    }

    fn draw_storage(&mut self, f: &mut Frame, area: Rect) {
        let header = Row::new(vec![
            Cell::from("  Filesystem"),
            Cell::from("Size"),
            Cell::from("Used"),
            Cell::from("Avail"),
            Cell::from("Use%"),
            Cell::from("Mount"),
        ])
        .style(Style::default().fg(theme::ACCENT).add_modifier(Modifier::BOLD));

        let rows: Vec<Row> = self
            .storage
            .iter()
            .map(|s| {
                let pct_val: u32 = s.use_pct.trim_end_matches('%').parse().unwrap_or(0);
                let pct_color = if pct_val >= 90 {
                    theme::RED
                } else if pct_val >= 75 {
                    theme::YELLOW
                } else {
                    theme::GREEN
                };
                Row::new(vec![
                    Cell::from(format!("  {}", s.filesystem)).style(Style::default().fg(theme::TEXT)),
                    Cell::from(s.size.as_str()).style(Style::default().fg(theme::DIM)),
                    Cell::from(s.used.as_str()).style(Style::default().fg(theme::DIM)),
                    Cell::from(s.avail.as_str()).style(Style::default().fg(theme::DIM)),
                    Cell::from(s.use_pct.as_str()).style(Style::default().fg(pct_color)),
                    Cell::from(s.mount.as_str()).style(Style::default().fg(theme::MUTED)),
                ])
            })
            .collect();

        let widths = [
            Constraint::Min(20),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(6),
            Constraint::Min(12),
        ];

        let table = Table::new(rows, widths)
            .header(header)
            .block(Block::default().borders(Borders::NONE).style(Style::default().bg(theme::BG)))
            .row_highlight_style(Style::default().bg(theme::HIGHLIGHT).fg(theme::TEXT));

        f.render_stateful_widget(table, area, &mut self.storage_state);
    }

    pub fn handle_mouse_click(&mut self, row: u16, _col: u16, _area: &Rect) {
        // Sub-tab header is row 0
        if row == 0 {
            let third = _area.width / 3;
            if _col < third {
                self.sub_tab = SubTab::Partitions;
            } else if _col < third * 2 {
                self.sub_tab = SubTab::Nodes;
            } else {
                self.sub_tab = SubTab::Storage;
            }
            return;
        }
        // Data rows (after header row at 0 and table header at 1)
        if row >= 2 {
            let idx = (row - 2) as usize;
            match self.sub_tab {
                SubTab::Partitions => {
                    if idx < self.partitions.len() {
                        self.part_state.select(Some(idx));
                    }
                }
                SubTab::Nodes => {
                    if idx < self.nodes.len() {
                        self.node_state.select(Some(idx));
                    }
                }
                SubTab::Storage => {
                    if idx < self.storage.len() {
                        self.storage_state.select(Some(idx));
                    }
                }
            }
        }
    }

    pub fn scroll_down(&mut self) {
        match self.sub_tab {
            SubTab::Partitions => {
                let len = self.partitions.len();
                if len > 0 {
                    let i = self.part_state.selected().map_or(0, |i| (i + 1).min(len - 1));
                    self.part_state.select(Some(i));
                }
            }
            SubTab::Nodes => {
                let len = self.nodes.len();
                if len > 0 {
                    let i = self.node_state.selected().map_or(0, |i| (i + 1).min(len - 1));
                    self.node_state.select(Some(i));
                }
            }
            SubTab::Storage => {
                let len = self.storage.len();
                if len > 0 {
                    let i = self.storage_state.selected().map_or(0, |i| (i + 1).min(len - 1));
                    self.storage_state.select(Some(i));
                }
            }
        }
    }

    pub fn scroll_up(&mut self) {
        match self.sub_tab {
            SubTab::Partitions => {
                let i = self.part_state.selected().map_or(0, |i| i.saturating_sub(1));
                self.part_state.select(Some(i));
            }
            SubTab::Nodes => {
                let i = self.node_state.selected().map_or(0, |i| i.saturating_sub(1));
                self.node_state.select(Some(i));
            }
            SubTab::Storage => {
                let i = self.storage_state.selected().map_or(0, |i| i.saturating_sub(1));
                self.storage_state.select(Some(i));
            }
        }
    }
}
