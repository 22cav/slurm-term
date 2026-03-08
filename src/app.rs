use std::io;
use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind, MouseButton, EnableMouseCapture, DisableMouseCapture, EnableBracketedPaste, DisableBracketedPaste};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::execute;
use ratatui::prelude::*;
use ratatui::widgets::*;

use crate::config::SlurmTermConfig;
use crate::slurm_api::SlurmController;
use crate::theme;
use crate::tabs::monitor::{self, MonitorState};
use crate::tabs::composer::{self, ComposerState};
use crate::tabs::hardware::{self, HardwareState};
use crate::tabs::history::{self, HistoryState};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TabId {
    Monitor,
    Composer,
    Hardware,
    History,
}

impl TabId {
    fn label(&self) -> &'static str {
        match self {
            Self::Monitor => "Jobs",
            Self::Composer => "Submit",
            Self::Hardware => "Cluster",
            Self::History => "History",
        }
    }

    fn all() -> &'static [TabId] {
        &[
            Self::Monitor,
            Self::Composer,
            Self::Hardware,
            Self::History,
        ]
    }
}

/// Status notification shown briefly at the bottom.
pub struct StatusMsg {
    pub text: String,
    pub expires: Instant,
}

pub struct App {
    active_tab: TabId,
    slurm: Box<dyn SlurmController + Send>,
    config: SlurmTermConfig,
    cluster_name: String,
    user: String,
    hostname: String,
    status: Option<StatusMsg>,
    should_quit: bool,

    pub monitor: MonitorState,
    pub composer: ComposerState,
    pub hardware: HardwareState,
    pub history: HistoryState,

    // Confirm dialog
    pub confirm: Option<ConfirmDialog>,

    // Layout regions for mouse hit-testing
    tab_rects: Vec<(TabId, Rect)>,
    content_area: Rect,
    last_area: Rect,
    hostname_rect: Rect,
    show_hostname: bool,
}

pub struct ConfirmDialog {
    pub message: String,
    pub on_yes: Box<dyn FnOnce(&mut App)>,
}

impl App {
    pub fn run(
        slurm: Box<dyn SlurmController + Send>,
        config: SlurmTermConfig,
        load_file: Option<String>,
    ) -> io::Result<()> {
        // Install a panic hook that restores the terminal before printing the error.
        // Without this, a panic leaves raw mode and mouse capture enabled, which
        // causes the terminal to echo escape codes for every mouse movement.
        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            let _ = terminal::disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture, DisableBracketedPaste);
            original_hook(panic_info);
        }));

        terminal::enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture, EnableBracketedPaste)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let cluster_name = slurm.get_cluster_name();
        let user = slurm.current_user();
        let hostname = std::process::Command::new("hostname")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        let history_window = config.history_window.clone();
        // Parse "now-Xdays" format to extract days count
        let history_days: u32 = history_window
            .strip_prefix("now-")
            .and_then(|s| s.strip_suffix("days"))
            .and_then(|s| s.parse().ok())
            .unwrap_or(7);
        let mut app = App {
            active_tab: TabId::Monitor,
            slurm,
            config,
            cluster_name,
            user,
            hostname,
            status: None,
            should_quit: false,
            monitor: MonitorState::default(),
            composer: ComposerState::new(),
            hardware: HardwareState::new(),
            history: HistoryState::new(history_days),
            confirm: None,
            tab_rects: Vec::new(),
            content_area: Rect::default(),
            last_area: Rect::default(),
            hostname_rect: Rect::default(),
            show_hostname: false,
        };

        // Initial data fetch
        app.poll_all();

        // Load .sbatch file if provided via --file
        if let Some(ref path) = load_file {
            match crate::sbatch_parser::parse_sbatch_file(path) {
                Ok(state) => {
                    app.composer.set_form_state(&state);
                    app.active_tab = TabId::Composer;
                    app.set_status(&format!("Loaded {path}"));
                }
                Err(e) => {
                    app.set_status(&format!("! {e}"));
                }
            }
        }

        let tick_rate = Duration::from_millis(250);
        let mut last_poll = Instant::now();
        let mut last_inspector_poll = Instant::now();

        loop {
            terminal.draw(|f| app.draw(f))?;

            let timeout = tick_rate
                .checked_sub(last_poll.elapsed())
                .unwrap_or(Duration::ZERO);

            if event::poll(timeout)? {
                match event::read()? {
                    Event::Key(key) => app.handle_key(key),
                    Event::Mouse(mouse) => app.handle_mouse(mouse),
                    Event::Paste(text) => app.handle_paste(&text),
                    _ => {}
                }
            }

            if last_poll.elapsed() >= Duration::from_secs_f64(app.poll_interval()) {
                app.poll_active_tab();
                last_poll = Instant::now();
                last_inspector_poll = Instant::now();
            }

            // Fast inspector log refresh when following
            if app.active_tab == TabId::Monitor {
                if let Some(ref mut insp) = app.monitor.inspector {
                    if insp.log_follow
                        && last_inspector_poll.elapsed()
                            >= Duration::from_secs_f64(app.config.inspector_poll_interval)
                    {
                        insp.load_log_tail();
                        last_inspector_poll = Instant::now();
                    }
                }
            }

            if app.should_quit {
                break;
            }
        }

        terminal::disable_raw_mode()?;
        execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture, DisableBracketedPaste)?;
        Ok(())
    }

    fn poll_interval(&self) -> f64 {
        match self.active_tab {
            TabId::Monitor => self.config.monitor_poll_interval,
            TabId::Hardware => self.config.hardware_poll_interval,
            TabId::History => self.config.history_poll_interval,
            _ => 5.0,
        }
    }

    fn poll_all(&mut self) {
        self.monitor.poll(&*self.slurm);
        self.hardware.poll(&*self.slurm);
        self.history.poll(&*self.slurm);
        if self.composer.partitions.is_empty() {
            self.composer.partitions = self.slurm.get_partitions();
        }
        // Ensure preview text is initialized
        if self.composer.preview_text.is_empty() {
            self.composer.sync_preview_from_form();
        }
    }

    fn poll_active_tab(&mut self) {
        match self.active_tab {
            TabId::Monitor => self.monitor.poll(&*self.slurm),
            TabId::Hardware => self.hardware.poll(&*self.slurm),
            TabId::History => self.history.poll(&*self.slurm),
            _ => {}
        }
    }

    fn set_status(&mut self, msg: &str) {
        self.status = Some(StatusMsg {
            text: msg.to_string(),
            expires: Instant::now() + Duration::from_secs(5),
        });
    }

    fn draw(&mut self, f: &mut Frame) {
        self.last_area = f.area();
        // Full background
        f.render_widget(Block::default().style(Style::default().bg(theme::BG)), f.area());

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1), // tab bar (merged with header)
                Constraint::Min(0),   // content
                Constraint::Length(1), // status + help bar
            ])
            .split(f.area());

        self.content_area = chunks[1];

        // Combined header + tab bar
        let mut tab_spans: Vec<Span> = vec![
            Span::styled(" slurm-term ", Style::default().fg(theme::BG).bg(theme::ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled(" ", Style::default().bg(theme::BG)),
        ];

        // Track tab positions for mouse clicking
        self.tab_rects.clear();
        let mut x_offset: u16 = 13; // after " slurm-term  "

        for (i, t) in TabId::all().iter().enumerate() {
            let num = format!(" {}", i + 1);
            let label = format!(" {} ", t.label());
            let full_len = (num.len() + label.len()) as u16;

            let tab_rect = Rect::new(chunks[0].x + x_offset, chunks[0].y, full_len, 1);
            self.tab_rects.push((*t, tab_rect));
            x_offset += full_len;

            if *t == self.active_tab {
                tab_spans.push(Span::styled(num, Style::default().fg(theme::ACCENT).bg(theme::SURFACE).add_modifier(Modifier::BOLD)));
                tab_spans.push(Span::styled(label, Style::default().fg(theme::TEXT).bg(theme::SURFACE).add_modifier(Modifier::BOLD)));
            } else {
                tab_spans.push(Span::styled(num, Style::default().fg(theme::MUTED)));
                tab_spans.push(Span::styled(label, Style::default().fg(theme::DIM)));
            }
        }

        // Right-align cluster + user + node type info
        let is_login = self.hostname.contains("login") || self.hostname.contains("head") || self.hostname.contains("front");
        let node_tag = if is_login { "login" } else { "compute" };
        let node_display = if self.show_hostname {
            format!("{} ({})", self.hostname, node_tag)
        } else {
            node_tag.to_string()
        };
        let right_text = format!("  {} │ {} │ {} ", self.cluster_name, self.user, node_display);
        let right_len = right_text.len() as u16;
        let pad = chunks[0].width.saturating_sub(x_offset + right_len);
        tab_spans.push(Span::styled(" ".repeat(pad as usize), Style::default()));
        tab_spans.push(Span::styled(
            format!("  {} │ {} │ ", self.cluster_name, self.user),
            Style::default().fg(theme::MUTED),
        ));
        let node_color = if is_login { theme::GREEN } else { theme::YELLOW };
        let node_span_len = node_display.len() as u16 + 1; // +1 for trailing space
        let node_x = chunks[0].x + chunks[0].width - node_span_len;
        self.hostname_rect = Rect::new(node_x, chunks[0].y, node_span_len, 1);
        tab_spans.push(Span::styled(
            node_display,
            Style::default().fg(node_color),
        ));
        tab_spans.push(Span::styled(" ", Style::default()));

        f.render_widget(
            Paragraph::new(Line::from(tab_spans)).style(Style::default().bg(theme::BG)),
            chunks[0],
        );

        // Content
        match self.active_tab {
            TabId::Monitor => self.monitor.draw(f, chunks[1]),
            TabId::Composer => self.composer.draw(f, chunks[1]),
            TabId::Hardware => self.hardware.draw(f, chunks[1]),
            TabId::History => self.history.draw(f, chunks[1]),
        }

        // Bottom bar: status left, shortcuts right
        let status_text = if let Some(ref s) = self.status {
            if Instant::now() < s.expires {
                s.text.clone()
            } else {
                self.status_default()
            }
        } else {
            self.status_default()
        };

        let help_spans = self.help_spans();
        let status_span = Span::styled(format!(" {status_text}"), Style::default().fg(theme::DIM));

        let mut bottom_spans = vec![status_span];
        // Calculate remaining width for right-aligned help
        let status_len = status_text.len() as u16 + 1;
        let help_text_len: u16 = help_spans.iter().map(|s| s.width() as u16).sum();
        let gap = chunks[2].width.saturating_sub(status_len + help_text_len + 1);
        bottom_spans.push(Span::styled(" ".repeat(gap as usize), Style::default()));
        bottom_spans.extend(help_spans);

        f.render_widget(
            Paragraph::new(Line::from(bottom_spans))
                .style(Style::default().bg(theme::SURFACE)),
            chunks[2],
        );

        // Confirm dialog overlay
        if let Some(ref cd) = self.confirm {
            let area = centered_rect(50, 7, f.area());
            let block = Block::default()
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(theme::YELLOW))
                .title(Span::styled(" Confirm ", Style::default().fg(theme::YELLOW).add_modifier(Modifier::BOLD)))
                .style(Style::default().bg(theme::SURFACE));
            let inner = block.inner(area);
            f.render_widget(Clear, area);
            f.render_widget(block, area);
            f.render_widget(
                Paragraph::new(vec![
                    Line::from(Span::styled(cd.message.clone(), Style::default().fg(theme::TEXT))),
                    Line::from(""),
                    Line::from(vec![
                        Span::styled("[y]", Style::default().fg(theme::GREEN).add_modifier(Modifier::BOLD)),
                        Span::styled(" Yes   ", Style::default().fg(theme::DIM)),
                        Span::styled("[n/Esc]", Style::default().fg(theme::RED).add_modifier(Modifier::BOLD)),
                        Span::styled(" No", Style::default().fg(theme::DIM)),
                    ]),
                ])
                .alignment(Alignment::Center),
                inner,
            );
        }
    }

    fn help_spans(&self) -> Vec<Span<'static>> {
        let key = |k: &'static str| -> Span<'static> {
            Span::styled(format!(" {k} "), Style::default().fg(theme::BG).bg(theme::MUTED))
        };
        let desc = |d: &'static str| -> Span<'static> {
            Span::styled(format!(" {d}"), Style::default().fg(theme::DIM))
        };
        let sep = || -> Span<'static> {
            Span::styled(" ", Style::default())
        };

        let mut spans: Vec<Span<'static>> = Vec::new();

        match self.active_tab {
            TabId::Monitor => {
                if self.monitor.inspector.is_some() {
                    spans.extend(vec![
                        key("Esc"), desc("Back"), sep(),
                        key("Tab"), desc("Section"), sep(),
                        key("e"), desc("Logs"), sep(),
                        key("f"), desc("Follow"), sep(),
                        key("s"), desc("Resubmit"),
                    ]);
                } else {
                    spans.extend(vec![
                        key("/"), desc("Search"), sep(),
                        key("Enter"), desc("Inspect"), sep(),
                        key("Space"), desc("Select"), sep(),
                        key("s"), desc("Sort"), sep(),
                        key("k"), desc("Kill"), sep(),
                        key("r"), desc("Refresh"),
                    ]);
                }
            }
            TabId::Composer => {
                spans.extend(vec![
                    key("Tab"), desc("Pane"), sep(),
                    key("Enter"), desc("Edit"), sep(),
                    key("?"), desc("Help"), sep(),
                    key("a"), desc("Add Param"), sep(),
                    key("^O"), desc("Load File"), sep(),
                    key("^Y"), desc("Copy"), sep(),
                    key("^S"), desc("Submit"),
                ]);
            }
            TabId::Hardware => {
                spans.extend(vec![
                    key("Tab"), desc("View"), sep(),
                    key("s"), desc("Sort"), sep(),
                    key("r"), desc("Refresh"),
                ]);
            }
            TabId::History => {
                spans.extend(vec![
                    key("Enter"), desc("Inspect"), sep(),
                    key("</>"), desc("Window"), sep(),
                    key("s"), desc("Sort"), sep(),
                    key("r"), desc("Refresh"),
                ]);
            }
        }

        spans.extend(vec![sep(), key("q"), desc("Quit")]);
        spans
    }

    fn status_default(&self) -> String {
        match self.active_tab {
            TabId::Monitor => {
                if let Some(ref inspector) = self.monitor.inspector {
                    if let Some(ref jid) = inspector.job_id {
                        return format!("Inspecting job {jid}");
                    }
                }
                let n = self.monitor.jobs.len();
                let sel = self.monitor.selected.len();
                let mut s = format!("{n} job{}", if n == 1 { "" } else { "s" });
                if sel > 0 {
                    s.push_str(&format!(" ({sel} selected)"));
                }
                s
            }
            TabId::Hardware => {
                let np = self.hardware.partitions.len();
                let nn = self.hardware.nodes.len();
                format!("{np} partition entries, {nn} nodes")
            }
            TabId::History => {
                let n = self.history.rows.len();
                format!("{n} completed jobs")
            }
            _ => String::new(),
        }
    }

    fn handle_key(&mut self, key: KeyEvent) {
        // Confirm dialog takes priority
        if self.confirm.is_some() {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    let dialog = self.confirm.take().unwrap();
                    (dialog.on_yes)(self);
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    self.confirm = None;
                }
                _ => {}
            }
            return;
        }

        // Global keys
        match key.code {
            // Ctrl+C always quits
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true;
                return;
            }
            KeyCode::Char('q') if key.modifiers.is_empty() || key.modifiers == KeyModifiers::NONE => {
                // Don't quit if in search mode or text input
                if self.is_text_input_active() {
                    match self.active_tab {
                        TabId::Monitor => { self.monitor.handle_key(key, &*self.slurm); }
                        TabId::Composer => { self.composer.handle_key(key, &*self.slurm); }
                        _ => {}
                    }
                    return;
                }
                self.should_quit = true;
                return;
            }
            KeyCode::Char('1') if !self.is_text_input_active() => {
                self.active_tab = TabId::Monitor;
                return;
            }
            KeyCode::Char('2') if !self.is_text_input_active() => {
                self.active_tab = TabId::Composer;
                if self.composer.partitions.is_empty() {
                    self.composer.partitions = self.slurm.get_partitions();
                }
                return;
            }
            KeyCode::Char('3') if !self.is_text_input_active() => {
                self.active_tab = TabId::Hardware;
                return;
            }
            KeyCode::Char('4') if !self.is_text_input_active() => {
                self.active_tab = TabId::History;
                return;
            }
            _ => {}
        }

        // Tab-specific keys
        match self.active_tab {
            TabId::Monitor => {
                let action = self.monitor.handle_key(key, &*self.slurm);
                match action {
                    monitor::Action::None => {}
                    monitor::Action::Refresh => {
                        self.monitor.poll(&*self.slurm);
                    }
                    monitor::Action::CancelJobs(ids) => {
                        let n = ids.len();
                        let msg = if n == 1 {
                            format!("Cancel job {}?", ids[0])
                        } else {
                            format!("Cancel {n} selected jobs?")
                        };
                        self.confirm = Some(ConfirmDialog {
                            message: msg,
                            on_yes: Box::new(move |app| {
                                let mut ok = 0;
                                for id in &ids {
                                    if app.slurm.cancel_job(id) {
                                        ok += 1;
                                    }
                                }
                                app.monitor.selected.clear();
                                app.set_status(&format!("Cancelled {ok} job(s)"));
                                app.monitor.poll(&*app.slurm);
                            }),
                        });
                    }
                    monitor::Action::HoldJobs(ids) => {
                        for id in &ids {
                            self.slurm.hold_job(id);
                        }
                        self.set_status(&format!("Held {} job(s)", ids.len()));
                        self.monitor.poll(&*self.slurm);
                    }
                    monitor::Action::ReleaseJobs(ids) => {
                        for id in &ids {
                            self.slurm.release_job(id);
                        }
                        self.set_status(&format!("Released {} job(s)", ids.len()));
                        self.monitor.poll(&*self.slurm);
                    }
                    monitor::Action::Resubmit(form_state) => {
                        self.composer.set_form_state(&form_state);
                        self.active_tab = TabId::Composer;
                        self.set_status("Job parameters loaded for resubmission");
                    }
                }
            }
            TabId::Composer => {
                let action = self.composer.handle_key(key, &*self.slurm);
                match action {
                    composer::Action::None => {}
                    composer::Action::Submit(params, script, body) => {
                        let result = if !script.is_empty() {
                            self.slurm.submit_job(&script, &params)
                        } else if !body.is_empty() {
                            // Write script body to temp file and submit
                            let tmp = std::env::temp_dir().join(
                                format!("slurm-term-{}.sh", std::process::id())
                            );
                            std::fs::write(&tmp, &body)
                                .map_err(|e| format!("Failed to write temp script: {e}"))
                                .and_then(|_| {
                                    let r = self.slurm.submit_job(
                                        &tmp.to_string_lossy(), &params
                                    );
                                    let _ = std::fs::remove_file(&tmp);
                                    r
                                })
                        } else {
                            Err("No script content to submit".into())
                        };
                        match result {
                            Ok(id) => {
                                self.set_status(&format!("Job {id} submitted"));
                                self.active_tab = TabId::Monitor;
                                self.monitor.poll(&*self.slurm);
                            }
                            Err(e) => self.set_status(&format!("Submit failed: {e}")),
                        }
                    }
                    composer::Action::Status(msg) => {
                        self.set_status(&msg);
                    }
                }
            }
            TabId::Hardware => {
                let action = self.hardware.handle_key(key, &*self.slurm);
                if let hardware::Action::Refresh = action {
                    self.hardware.poll(&*self.slurm);
                }
            }
            TabId::History => {
                let action = self.history.handle_key(key, &*self.slurm);
                match action {
                    history::Action::None => {}
                    history::Action::Inspect(job_id) => {
                        // Open in monitor's inline inspector
                        let mut insp = crate::tabs::inspector::InspectorState::new();
                        insp.load_job(&job_id, &*self.slurm);
                        self.monitor.inspector = Some(insp);
                        self.active_tab = TabId::Monitor;
                    }
                    history::Action::Refresh => {
                        self.history.poll(&*self.slurm);
                    }
                }
            }
        }
    }

    fn is_text_input_active(&self) -> bool {
        match self.active_tab {
            TabId::Monitor => self.monitor.search_active || self.monitor.inspector.is_some(),
            TabId::Composer => {
                self.composer.editing
                    || self.composer.template_dialog.is_some()
                    || self.composer.help_overlay
                    || self.composer.add_param_dialog.is_some()
                    || self.composer.load_file_dialog.is_some()
            }
            _ => false,
        }
    }

    fn handle_paste(&mut self, text: &str) {
        if self.active_tab == TabId::Composer {
            self.composer.handle_paste(text);
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent) {
        let col = mouse.column;
        let row = mouse.row;

        match mouse.kind {
            MouseEventKind::Moved => {
                let r = self.hostname_rect;
                self.show_hostname = col >= r.x && col < r.x + r.width
                    && row >= r.y && row < r.y + r.height;
            }
            MouseEventKind::Down(MouseButton::Left) => {
                // Check tab bar clicks
                for &(tab, rect) in &self.tab_rects {
                    if col >= rect.x && col < rect.x + rect.width && row >= rect.y && row < rect.y + rect.height {
                        self.active_tab = tab;
                        if tab == TabId::Composer && self.composer.partitions.is_empty() {
                            self.composer.partitions = self.slurm.get_partitions();
                        }
                        return;
                    }
                }

                // Delegate to content area
                if col >= self.content_area.x
                    && col < self.content_area.x + self.content_area.width
                    && row >= self.content_area.y
                    && row < self.content_area.y + self.content_area.height
                {
                    let rel_row = row.saturating_sub(self.content_area.y);
                    let rel_col = col.saturating_sub(self.content_area.x);
                    match self.active_tab {
                        TabId::Monitor => {
                            self.monitor.handle_mouse_click(rel_row, rel_col, &self.content_area);
                        }
                        TabId::Hardware => {
                            self.hardware.handle_mouse_click(rel_row, rel_col, &self.content_area);
                        }
                        TabId::History => {
                            self.history.handle_mouse_click(rel_row, rel_col, &self.content_area);
                        }
                        TabId::Composer => {
                            self.composer.handle_mouse_click(rel_row, rel_col, &self.content_area);
                        }
                    }
                }
            }
            MouseEventKind::ScrollDown => {
                if col >= self.content_area.x
                    && col < self.content_area.x + self.content_area.width
                    && row >= self.content_area.y
                    && row < self.content_area.y + self.content_area.height
                {
                    match self.active_tab {
                        TabId::Monitor => self.monitor.scroll_down(),
                        TabId::Hardware => self.hardware.scroll_down(),
                        TabId::History => self.history.scroll_down(),
                        TabId::Composer => self.composer.scroll_down(),
                    }
                }
            }
            MouseEventKind::ScrollUp => {
                if col >= self.content_area.x
                    && col < self.content_area.x + self.content_area.width
                    && row >= self.content_area.y
                    && row < self.content_area.y + self.content_area.height
                {
                    match self.active_tab {
                        TabId::Monitor => self.monitor.scroll_up(),
                        TabId::Hardware => self.hardware.scroll_up(),
                        TabId::History => self.history.scroll_up(),
                        TabId::Composer => self.composer.scroll_up(),
                    }
                }
            }
            _ => {}
        }
    }
}

pub fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let popup_width = area.width * percent_x / 100;
    let x = (area.width.saturating_sub(popup_width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    Rect::new(
        area.x + x,
        area.y + y,
        popup_width.min(area.width),
        height.min(area.height),
    )
}
