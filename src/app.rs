use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Local;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use crate::collectors::Sampler;
use crate::history::pdf::{write_report, ReportInput};
use crate::history::raw::{self, Recorder};
use crate::history::History;
use crate::model::{fmt_wh, Sample, Snapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Overview,
    Cpu,
    Power,
    History,
    Help,
}

impl Tab {
    pub const ALL: [Tab; 5] = [Tab::Overview, Tab::Cpu, Tab::Power, Tab::History, Tab::Help];

    pub fn title(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Cpu => "CPU",
            Tab::Power => "Power",
            Tab::History => "History",
            Tab::Help => "Help",
        }
    }

    pub fn index(self) -> usize {
        match self {
            Tab::Overview => 0,
            Tab::Cpu => 1,
            Tab::Power => 2,
            Tab::History => 3,
            Tab::Help => 4,
        }
    }

    pub fn from_index(index: usize) -> Self {
        Self::ALL[index.min(Self::ALL.len() - 1)]
    }
}

#[derive(Clone, Debug)]
pub enum Action {
    SwitchTab(Tab),
    ToggleRecord,
    ExportPdf,
    TogglePause,
    ShowHelp,
    SetWindow(u64),
    ModalSelect(usize),
    ModalCancel,
}

#[derive(Clone, Debug)]
pub enum ExportSource {
    Session,
    RawFile(PathBuf),
}

#[derive(Clone, Debug)]
pub struct ExportOption {
    pub label: String,
    pub enabled: bool,
    pub source: Option<ExportSource>,
}

#[derive(Clone, Debug)]
pub struct ExportModal {
    pub options: Vec<ExportOption>,
    pub selected: usize,
    pub output_path: PathBuf,
}

#[derive(Clone, Debug)]
pub enum Modal {
    Export(ExportModal),
    Error { title: String, message: String },
}

pub struct App {
    pub tab: Tab,
    pub prev_tab: Tab,
    pub sampler: Sampler,
    pub history: History,
    pub snapshot: Option<Snapshot>,
    pub recorder: Recorder,
    pub modal: Option<Modal>,
    pub hit_zones: Vec<(Rect, Action)>,
    pub content_rect: Rect,
    pub paused: bool,
    pub should_quit: bool,
    pub needs_redraw: bool,
    pub warnings: Vec<String>,
    pub status: String,
    pub interval_ms: u64,
    pub window_secs: u64,
    pub mouse_enabled: bool,
    pub ascii: bool,
    pub power_scroll: usize,
    pub help_scroll: usize,
    pub report_on_exit: Option<PathBuf>,
    pub last_export: Option<PathBuf>,
    pub last_raw: Option<PathBuf>,
    pub mouse_toggle_requested: Option<bool>,
}

pub struct AppOptions {
    pub interval_ms: u64,
    pub history_secs: u64,
    pub window_secs: u64,
    pub mouse_enabled: bool,
    pub ascii: bool,
    pub raw_path: Option<PathBuf>,
    pub report_path: Option<PathBuf>,
}

impl App {
    pub fn new(sampler: Sampler, options: AppOptions) -> Self {
        let warnings = sampler.warnings().to_vec();
        let mut app = Self {
            tab: Tab::Overview,
            prev_tab: Tab::Overview,
            sampler,
            history: History::new(Duration::from_secs(options.history_secs.max(1))),
            snapshot: None,
            recorder: Recorder::new(),
            modal: None,
            hit_zones: Vec::new(),
            content_rect: Rect::default(),
            paused: false,
            should_quit: false,
            needs_redraw: true,
            warnings,
            status: "warming up...".to_string(),
            interval_ms: options.interval_ms.max(100),
            window_secs: options.window_secs,
            mouse_enabled: options.mouse_enabled,
            ascii: options.ascii,
            power_scroll: 0,
            help_scroll: 0,
            report_on_exit: options.report_path,
            last_export: None,
            last_raw: None,
            mouse_toggle_requested: None,
        };
        if let Some(path) = options.raw_path {
            if let Err(err) = app.start_recording_at(path) {
                app.warnings.push(err);
            }
        }
        app
    }

    pub fn tick(&mut self) {
        if self.paused {
            return;
        }
        let snapshot = self.sampler.tick();
        self.history.push(snapshot.sample.clone());
        if self.recorder.is_recording() {
            if let Err(err) = self.recorder.write(&snapshot) {
                self.warnings.push(format!("raw write failed: {err}"));
                self.recorder.stop();
                self.status = "recording stopped after write error".to_string();
            }
        }
        self.snapshot = Some(snapshot);
        if self.status.starts_with("warming up") {
            let components_ready = self
                .snapshot
                .as_ref()
                .is_some_and(|s| s.sample.cpu_w.is_some());
            if components_ready || self.history.len() > 1 {
                self.status = "live".to_string();
            } else {
                self.status = "warming up sensors...".to_string();
            }
        }
        self.needs_redraw = true;
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
            self.should_quit = true;
            return;
        }
        if let Some(modal) = self.modal.take() {
            match modal {
                Modal::Export(mut export) => match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => {}
                    KeyCode::Up | KeyCode::Char('k') => {
                        export.selected = export.selected.saturating_sub(1);
                        self.modal = Some(Modal::Export(export));
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        export.selected =
                            (export.selected + 1).min(export.options.len().saturating_sub(1));
                        self.modal = Some(Modal::Export(export));
                    }
                    KeyCode::Enter => {
                        self.activate_export_option(&export, export.selected);
                    }
                    _ => self.modal = Some(Modal::Export(export)),
                },
                Modal::Error { .. } => {
                    if matches!(key.code, KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q')) {
                        // closed
                    } else {
                        self.modal = Some(modal);
                    }
                }
            }
            self.needs_redraw = true;
            return;
        }

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') => self.toggle_help(),
            KeyCode::Esc => {
                if self.tab == Tab::Help {
                    self.tab = self.prev_tab;
                }
            }
            KeyCode::Tab => self.next_tab(1),
            KeyCode::BackTab => self.next_tab(-1),
            KeyCode::Char('1') => self.switch_tab(Tab::Overview),
            KeyCode::Char('2') => self.switch_tab(Tab::Cpu),
            KeyCode::Char('3') => self.switch_tab(Tab::Power),
            KeyCode::Char('4') => self.switch_tab(Tab::History),
            KeyCode::Char('5') => self.switch_tab(Tab::Help),
            KeyCode::Char('h') => self.next_tab(-1),
            KeyCode::Char('l') => self.next_tab(1),
            KeyCode::Char('p') => self.toggle_pause(),
            KeyCode::Char('r') => self.toggle_recording(),
            KeyCode::Char('e') => self.open_export_modal(),
            KeyCode::Char('m') => {
                self.mouse_enabled = !self.mouse_enabled;
                self.mouse_toggle_requested = Some(self.mouse_enabled);
                self.status = if self.mouse_enabled {
                    "mouse capture on".to_string()
                } else {
                    "mouse capture off (terminal selection works)".to_string()
                };
            }
            KeyCode::Char('+') | KeyCode::Char('=') => self.change_interval(250),
            KeyCode::Char('-') | KeyCode::Char('_') => self.change_interval(-250),
            KeyCode::Char('t') => self.cycle_window(),
            KeyCode::Down | KeyCode::Char('j') => self.scroll(1),
            KeyCode::Up | KeyCode::Char('k') => self.scroll(-1),
            KeyCode::PageDown => self.scroll(10),
            KeyCode::PageUp => self.scroll(-10),
            KeyCode::Char('g') => self.scroll_to_top(),
            KeyCode::Char('G') => self.scroll_to_bottom(),
            _ => {}
        }
        self.needs_redraw = true;
    }

    pub fn on_mouse(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let pos = Position::new(mouse.column, mouse.row);
                if let Some((_, action)) = self
                    .hit_zones
                    .iter()
                    .find(|(rect, _)| rect.contains(pos))
                    .cloned()
                {
                    self.execute(action);
                }
            }
            MouseEventKind::ScrollUp => self.scroll_mouse(mouse, -3),
            MouseEventKind::ScrollDown => self.scroll_mouse(mouse, 3),
            _ => {}
        }
        self.needs_redraw = true;
    }

    fn scroll_mouse(&mut self, mouse: MouseEvent, delta: i32) {
        if !self
            .content_rect
            .contains(Position::new(mouse.column, mouse.row))
        {
            return;
        }
        if self.tab == Tab::History {
            self.cycle_window();
        } else {
            self.scroll(delta);
        }
    }

    fn execute(&mut self, action: Action) {
        match action {
            Action::SwitchTab(tab) => self.switch_tab(tab),
            Action::ToggleRecord => self.toggle_recording(),
            Action::ExportPdf => self.open_export_modal(),
            Action::TogglePause => self.toggle_pause(),
            Action::ShowHelp => self.toggle_help(),
            Action::SetWindow(secs) => {
                self.window_secs = secs;
                self.status = format!("window: {}", window_label(secs));
            }
            Action::ModalSelect(index) => {
                if let Some(Modal::Export(export)) = self.modal.as_ref() {
                    let export = export.clone();
                    self.activate_export_option(&export, index);
                }
            }
            Action::ModalCancel => {
                self.modal = None;
            }
        }
        self.needs_redraw = true;
    }

    fn switch_tab(&mut self, tab: Tab) {
        if tab != self.tab {
            self.prev_tab = self.tab;
            self.tab = tab;
        }
    }

    fn next_tab(&mut self, delta: i32) {
        let n = Tab::ALL.len() as i32;
        let idx = (self.tab.index() as i32 + delta).rem_euclid(n) as usize;
        self.switch_tab(Tab::from_index(idx));
    }

    fn toggle_help(&mut self) {
        if self.tab == Tab::Help {
            self.tab = self.prev_tab;
        } else {
            self.prev_tab = self.tab;
            self.tab = Tab::Help;
        }
    }

    fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        if !self.paused {
            self.sampler.reset();
            self.status = "resumed".to_string();
        } else {
            self.status = "paused".to_string();
        }
    }

    fn change_interval(&mut self, delta: i64) {
        let next = (self.interval_ms as i64 + delta).clamp(250, 10_000) as u64;
        if next != self.interval_ms {
            self.interval_ms = next;
            self.sampler.reset();
            self.status = format!("interval: {} ms", self.interval_ms);
        }
    }

    fn cycle_window(&mut self) {
        const WINDOWS: [u64; 5] = [60, 300, 900, 3600, 0];
        let pos = WINDOWS
            .iter()
            .position(|w| *w == self.window_secs)
            .unwrap_or(0);
        self.window_secs = WINDOWS[(pos + 1) % WINDOWS.len()];
        self.status = format!("window: {}", window_label(self.window_secs));
    }

    fn scroll(&mut self, delta: i32) {
        match self.tab {
            Tab::Power => {
                self.power_scroll = (self.power_scroll as i32 + delta).max(0) as usize;
            }
            Tab::Help => {
                self.help_scroll = (self.help_scroll as i32 + delta).max(0) as usize;
            }
            _ => {}
        }
    }

    fn scroll_to_top(&mut self) {
        self.power_scroll = 0;
        self.help_scroll = 0;
    }

    fn scroll_to_bottom(&mut self) {
        match self.tab {
            Tab::Power => self.power_scroll = 10_000,
            Tab::Help => self.help_scroll = 10_000,
            _ => {}
        }
    }

    fn toggle_recording(&mut self) {
        if self.recorder.is_recording() {
            let path = self.recorder.stop();
            if let Some(path) = path {
                self.last_raw = Some(path.clone());
                self.status = format!(
                    "recorded {} rows to {}",
                    self.recorder.rows(),
                    path.display()
                );
            } else {
                self.status = "recording stopped".to_string();
            }
        } else {
            let path = unique_path(default_raw_path());
            if let Err(err) = self.start_recording_at(path) {
                self.modal = Some(Modal::Error {
                    title: "Recording failed".to_string(),
                    message: err,
                });
            }
        }
    }

    fn start_recording_at(&mut self, path: PathBuf) -> Result<(), String> {
        let system = self.sampler.system().clone();
        self.recorder
            .start(path.clone(), &system)
            .map_err(|e| format!("cannot create {}: {e}", path.display()))?;
        self.last_raw = Some(path.clone());
        self.status = format!("recording to {}", path.display());
        Ok(())
    }

    fn open_export_modal(&mut self) {
        let mut options = Vec::new();
        if !self.history.is_empty() {
            options.push(ExportOption {
                label: format!("Session history ({} samples)", self.history.len()),
                enabled: true,
                source: Some(ExportSource::Session),
            });
        }
        if let Some(path) = self.recorder.last_path() {
            options.push(ExportOption {
                label: format!("Raw file: {}", path.display()),
                enabled: true,
                source: Some(ExportSource::RawFile(path.to_path_buf())),
            });
        }
        options.push(ExportOption {
            label: "Cancel".to_string(),
            enabled: true,
            source: None,
        });
        let output_path = self
            .report_on_exit
            .clone()
            .unwrap_or_else(|| unique_path(default_report_path()));
        self.modal = Some(Modal::Export(ExportModal {
            options,
            selected: 0,
            output_path,
        }));
    }

    fn activate_export_option(&mut self, export: &ExportModal, index: usize) {
        let Some(option) = export.options.get(index) else {
            return;
        };
        if !option.enabled {
            self.modal = Some(Modal::Export(export.clone()));
            return;
        }
        let Some(source) = option.source.clone() else {
            self.modal = None;
            return;
        };
        let result = self.export_pdf(&source, &export.output_path);
        match result {
            Ok(samples) => {
                self.last_export = Some(export.output_path.clone());
                self.status = format!(
                    "PDF written to {} ({} samples)",
                    export.output_path.display(),
                    samples
                );
                self.modal = None;
            }
            Err(err) => {
                self.modal = Some(Modal::Error {
                    title: "PDF export failed".to_string(),
                    message: err,
                });
            }
        }
    }

    fn export_pdf(&mut self, source: &ExportSource, output: &Path) -> Result<usize, String> {
        let (samples, source_label) = match source {
            ExportSource::Session => {
                let samples: Vec<Sample> = self.history.samples().cloned().collect();
                (samples, "session history".to_string())
            }
            ExportSource::RawFile(path) => {
                self.recorder
                    .flush()
                    .map_err(|e| format!("flush raw file: {e}"))?;
                let samples = raw::parse_file(path)?;
                (samples, format!("raw file {}", path.display()))
            }
        };
        if samples.is_empty() {
            return Err("no samples to export".to_string());
        }
        let rails = self
            .snapshot
            .as_ref()
            .map(|s| s.rails.clone())
            .unwrap_or_default();
        let system = self.sampler.system().clone();
        let input = ReportInput {
            samples: &samples,
            system: &system,
            rails: &rails,
            source_label,
        };
        write_report(output, &input)?;
        Ok(samples.len())
    }

    pub fn export_on_exit(&mut self) -> Result<Option<PathBuf>, String> {
        let Some(path) = self.report_on_exit.clone() else {
            return Ok(None);
        };
        if self.history.is_empty() {
            return Err("no session samples for --report".to_string());
        }
        let source = ExportSource::Session;
        self.export_pdf(&source, &path)?;
        self.last_export = Some(path.clone());
        Ok(Some(path))
    }

    pub fn session_summary(&self) -> String {
        let summary = self.history.summary(None);
        let duration = summary
            .duration_secs()
            .map(crate::model::fmt_duration_secs)
            .unwrap_or_else(|| "0:00".to_string());
        format!(
            "session: {} samples over {} | system energy {} | avg {} | peak {}",
            summary.count,
            duration,
            fmt_wh(summary.system.energy_wh),
            summary
                .system
                .avg
                .map(crate::model::fmt_power_val)
                .unwrap_or_else(|| "n/a".into()),
            summary
                .system
                .max
                .map(crate::model::fmt_power_val)
                .unwrap_or_else(|| "n/a".into())
        )
    }

    pub fn take_mouse_toggle(&mut self) -> Option<bool> {
        self.mouse_toggle_requested.take()
    }
}

pub fn window_label(secs: u64) -> String {
    match secs {
        0 => "All".to_string(),
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s => format!("{}h", s / 3600),
    }
}

pub fn default_raw_path() -> PathBuf {
    let stamp = Local::now().format("%Y%m%d-%H%M%S");
    PathBuf::from(format!("power-{stamp}.tsv"))
}

pub fn default_report_path() -> PathBuf {
    let stamp = Local::now().format("%Y%m%d-%H%M%S");
    PathBuf::from(format!("power-report-{stamp}.pdf"))
}

fn unique_path(path: PathBuf) -> PathBuf {
    if !path.exists() {
        return path;
    }
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "output".to_string());
    let ext = path
        .extension()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let parent = path.parent().map(|p| p.to_path_buf()).unwrap_or_default();
    for i in 1..1000 {
        let name = if ext.is_empty() {
            format!("{stem}-{i}")
        } else {
            format!("{stem}-{i}.{ext}")
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEventKind, KeyEventState};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::CONTROL,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn app() -> App {
        App::new(
            Sampler::demo(),
            AppOptions {
                interval_ms: 1000,
                history_secs: 60,
                window_secs: 60,
                mouse_enabled: true,
                ascii: false,
                raw_path: None,
                report_path: None,
            },
        )
    }

    #[test]
    fn tabs_cycle_and_direct_select() {
        let mut app = app();
        app.on_key(key(KeyCode::Tab));
        assert_eq!(app.tab, Tab::Cpu);
        app.on_key(key(KeyCode::Char('5')));
        assert_eq!(app.tab, Tab::Help);
        app.on_key(key(KeyCode::Esc));
        assert_eq!(app.tab, Tab::Cpu);
    }

    #[test]
    fn pause_resume_and_interval() {
        let mut app = app();
        app.on_key(key(KeyCode::Char('p')));
        assert!(app.paused);
        app.on_key(key(KeyCode::Char('p')));
        assert!(!app.paused);
        let before = app.interval_ms;
        app.on_key(key(KeyCode::Char('+')));
        assert!(app.interval_ms > before);
        app.on_key(key(KeyCode::Char('-')));
        assert_eq!(app.interval_ms, before);
    }

    #[test]
    fn window_cycles() {
        let mut app = app();
        app.on_key(key(KeyCode::Char('t')));
        assert_eq!(app.window_secs, 300);
    }

    #[test]
    fn export_modal_opens_with_session_option() {
        let mut app = app();
        app.tick();
        app.on_key(key(KeyCode::Char('e')));
        match app.modal {
            Some(Modal::Export(export)) => {
                assert!(export
                    .options
                    .iter()
                    .any(|o| o.label.contains("Session history")));
            }
            _ => panic!("expected export modal"),
        }
    }

    #[test]
    fn ctrl_c_quits_even_with_modal_open() {
        let mut app = app();
        app.tick();
        app.on_key(key(KeyCode::Char('e')));
        assert!(app.modal.is_some());
        app.on_key(ctrl(KeyCode::Char('c')));
        assert!(app.should_quit);
    }

    #[test]
    fn mouse_click_switches_tab() {
        let mut app = app();
        app.hit_zones = vec![(Rect::new(0, 0, 10, 2), Action::SwitchTab(Tab::Power))];
        app.on_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 3,
            row: 1,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.tab, Tab::Power);
    }
}
