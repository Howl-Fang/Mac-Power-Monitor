use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use chrono::{Local, NaiveDateTime};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

use crate::collectors::Sampler;
use crate::history::pdf::{write_report, ReportInput};
use crate::history::raw::{self, Recorder};
use crate::history::store::{self, HistoryStore};
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
    ExportSourcePrev,
    ExportSourceNext,
    ExportCell { window: usize, format: usize },
    ExportRun,
    ModalCancel,
}

pub const HISTORY_WINDOWS: [u64; 8] = [60, 300, 900, 3600, 21_600, 86_400, 172_800, 0];

pub const EXPORT_WINDOWS: [(&str, Option<u64>); 8] = [
    ("1 min", Some(60)),
    ("5 min", Some(300)),
    ("15 min", Some(900)),
    ("1 hour", Some(3600)),
    ("6 hours", Some(21_600)),
    ("24 hours", Some(86_400)),
    ("48 hours", Some(172_800)),
    ("All", None),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Pdf,
    Tsv,
}

impl ExportFormat {
    pub const ALL: [ExportFormat; 2] = [ExportFormat::Pdf, ExportFormat::Tsv];

    pub fn label(self) -> &'static str {
        match self {
            ExportFormat::Pdf => "PDF",
            ExportFormat::Tsv => "TSV",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Pdf => "pdf",
            ExportFormat::Tsv => "tsv",
        }
    }
}

#[derive(Clone, Debug)]
pub enum ExportSourceKind {
    Recent48h,
    Session,
    Recording(PathBuf),
}

#[derive(Clone, Debug)]
pub struct ExportSourceChoice {
    pub label: String,
    pub kind: ExportSourceKind,
}

#[derive(Clone, Debug)]
pub struct ExportModal {
    pub sources: Vec<ExportSourceChoice>,
    pub source: usize,
    pub window: usize,
    pub format: usize,
    pub stamp: String,
}

impl ExportModal {
    pub fn window_secs(&self) -> Option<u64> {
        EXPORT_WINDOWS.get(self.window).and_then(|(_, secs)| *secs)
    }

    pub fn format(&self) -> ExportFormat {
        ExportFormat::ALL
            .get(self.format)
            .copied()
            .unwrap_or(ExportFormat::Pdf)
    }

    pub fn output_path(&self) -> PathBuf {
        let prefix = match self.format() {
            ExportFormat::Pdf => "power-report",
            ExportFormat::Tsv => "power-history",
        };
        PathBuf::from(format!(
            "{prefix}-{}.{}",
            self.stamp,
            self.format().extension()
        ))
    }
}

#[derive(Clone, Debug)]
pub enum Modal {
    Export(ExportModal),
    Error { title: String, message: String },
}

pub struct HistoryCache {
    pub window_secs: u64,
    pub samples: VecDeque<Sample>,
}

pub struct App {
    pub tab: Tab,
    pub prev_tab: Tab,
    pub sampler: Sampler,
    pub history: History,
    pub history_secs: u64,
    pub history_cache: Option<HistoryCache>,
    pub store: Option<HistoryStore>,
    pub snapshot: Option<Snapshot>,
    pub recorder: Recorder,
    pub recording_finalize: bool,
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
    /// Directory for the always-on 48h history. `None` disables it.
    pub history_dir: Option<PathBuf>,
}

impl App {
    pub fn new(sampler: Sampler, options: AppOptions) -> Self {
        let mut warnings = sampler.warnings().to_vec();
        let store = match options.history_dir {
            Some(dir) => match HistoryStore::open(dir) {
                Ok(store) => Some(store),
                Err(err) => {
                    warnings.push(format!("rolling history unavailable: {err}"));
                    None
                }
            },
            None => None,
        };
        let mut app = Self {
            tab: Tab::Overview,
            prev_tab: Tab::Overview,
            sampler,
            history: History::new(Duration::from_secs(options.history_secs.max(1))),
            history_secs: options.history_secs.max(1),
            history_cache: None,
            store,
            snapshot: None,
            recorder: Recorder::new(),
            recording_finalize: false,
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
            if let Err(err) = app.start_recording_at(path, false) {
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
        self.update_history_cache(&snapshot.sample);
        let mut store_error = None;
        if let Some(store) = self.store.as_mut() {
            if let Err(err) = store.record(&snapshot, self.sampler.system()) {
                store_error = Some(format!("rolling history disabled: {err}"));
            }
        }
        if let Some(err) = store_error {
            self.store = None;
            self.warnings.push(err);
        }
        if self.recorder.is_recording() {
            if let Err(err) = self.recorder.write(&snapshot) {
                self.warnings.push(format!("raw write failed: {err}"));
                let path = self.recorder.stop();
                if self.recording_finalize {
                    if let Some(path) = path {
                        finalize_recording(&path);
                    }
                    self.recording_finalize = false;
                }
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
                        export.window = export.window.saturating_sub(1);
                        self.modal = Some(Modal::Export(export));
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        export.window =
                            (export.window + 1).min(EXPORT_WINDOWS.len().saturating_sub(1));
                        self.modal = Some(Modal::Export(export));
                    }
                    KeyCode::Left | KeyCode::Char('h') => {
                        export.format = export.format.saturating_sub(1);
                        self.modal = Some(Modal::Export(export));
                    }
                    KeyCode::Right | KeyCode::Char('l') => {
                        export.format =
                            (export.format + 1).min(ExportFormat::ALL.len().saturating_sub(1));
                        self.modal = Some(Modal::Export(export));
                    }
                    KeyCode::Tab => {
                        if !export.sources.is_empty() {
                            export.source = (export.source + 1) % export.sources.len();
                        }
                        self.modal = Some(Modal::Export(export));
                    }
                    KeyCode::BackTab => {
                        if !export.sources.is_empty() {
                            export.source =
                                (export.source + export.sources.len() - 1) % export.sources.len();
                        }
                        self.modal = Some(Modal::Export(export));
                    }
                    KeyCode::Enter => {
                        self.export_selected(&export);
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
                self.needs_redraw = true;
            }
            MouseEventKind::ScrollUp => {
                self.scroll_mouse(mouse, -3);
                self.needs_redraw = true;
            }
            MouseEventKind::ScrollDown => {
                self.scroll_mouse(mouse, 3);
                self.needs_redraw = true;
            }
            // Terminals can report hover/move and button-release events that
            // do not change the UI; repainting for them wastes CPU.
            _ => {}
        }
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
                self.refresh_history_cache();
            }
            Action::ExportSourcePrev | Action::ExportSourceNext => {
                if let Some(Modal::Export(mut export)) = self.modal.take() {
                    if !export.sources.is_empty() {
                        let len = export.sources.len();
                        export.source = if matches!(action, Action::ExportSourcePrev) {
                            (export.source + len - 1) % len
                        } else {
                            (export.source + 1) % len
                        };
                    }
                    self.modal = Some(Modal::Export(export));
                }
            }
            Action::ExportCell { window, format } => {
                if let Some(Modal::Export(mut export)) = self.modal.take() {
                    export.window = window.min(EXPORT_WINDOWS.len().saturating_sub(1));
                    export.format = format.min(ExportFormat::ALL.len().saturating_sub(1));
                    self.modal = Some(Modal::Export(export));
                }
            }
            Action::ExportRun => {
                if let Some(Modal::Export(export)) = self.modal.clone() {
                    self.export_selected(&export);
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
        let pos = HISTORY_WINDOWS
            .iter()
            .position(|w| *w == self.window_secs)
            .unwrap_or(0);
        self.window_secs = HISTORY_WINDOWS[(pos + 1) % HISTORY_WINDOWS.len()];
        self.status = format!("window: {}", window_label(self.window_secs));
        self.refresh_history_cache();
    }

    fn update_history_cache(&mut self, sample: &Sample) {
        let Some(cache) = self.history_cache.as_mut() else {
            return;
        };
        if cache.window_secs != self.window_secs {
            return;
        }
        cache.samples.push_back(sample.clone());
        let retention = if cache.window_secs == 0 {
            store::RETENTION_SECS
        } else {
            cache.window_secs
        };
        let cutoff = sample
            .ts
            .checked_sub(Duration::from_secs(retention))
            .unwrap_or(UNIX_EPOCH);
        while cache.samples.front().is_some_and(|front| front.ts < cutoff) {
            cache.samples.pop_front();
        }
    }

    fn refresh_history_cache(&mut self) {
        let needs_store =
            self.store.is_some() && (self.window_secs == 0 || self.window_secs > self.history_secs);
        if !needs_store {
            self.history_cache = None;
            return;
        }
        self.status = format!("loading {} history...", window_label(self.window_secs));
        let secs = if self.window_secs == 0 {
            None
        } else {
            Some(self.window_secs)
        };
        let Some(store) = self.store.as_mut() else {
            self.history_cache = None;
            return;
        };
        match store.samples_since(secs) {
            Ok(samples) => {
                let count = samples.len();
                self.history_cache = Some(HistoryCache {
                    window_secs: self.window_secs,
                    samples: samples.into(),
                });
                self.status = format!(
                    "history: {} ({} samples)",
                    window_label(self.window_secs),
                    count
                );
            }
            Err(err) => {
                self.history_cache = None;
                self.status = format!("history unavailable: {err}");
            }
        }
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
            let path = match path {
                Some(path) if self.recording_finalize => Some(finalize_recording(&path)),
                other => other,
            };
            self.recording_finalize = false;
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
            let dir = self.recording_dir();
            let path = match default_recording_path(&dir) {
                Ok(path) => path,
                Err(err) => {
                    self.modal = Some(Modal::Error {
                        title: "Recording failed".to_string(),
                        message: err,
                    });
                    return;
                }
            };
            if let Err(err) = self.start_recording_at(path, true) {
                self.modal = Some(Modal::Error {
                    title: "Recording failed".to_string(),
                    message: err,
                });
            }
        }
    }

    fn recording_dir(&self) -> PathBuf {
        self.store
            .as_ref()
            .map(|store| store.recordings_dir().to_path_buf())
            .unwrap_or_else(|| store::default_base_dir().join("recordings"))
    }

    fn start_recording_at(&mut self, path: PathBuf, finalize: bool) -> Result<(), String> {
        let system = self.sampler.system().clone();
        self.recorder
            .start(path.clone(), &system)
            .map_err(|e| format!("cannot create {}: {e}", path.display()))?;
        self.recording_finalize = finalize;
        self.last_raw = Some(path.clone());
        self.status = format!("recording to {}", path.display());
        Ok(())
    }

    fn export_sources(&self) -> Vec<ExportSourceChoice> {
        let mut sources = Vec::new();
        if self.store.is_some() {
            sources.push(ExportSourceChoice {
                label: "Last 48h".to_string(),
                kind: ExportSourceKind::Recent48h,
            });
        } else if !self.history.is_empty() {
            sources.push(ExportSourceChoice {
                label: "This session".to_string(),
                kind: ExportSourceKind::Session,
            });
        }
        if let Some(store) = &self.store {
            for path in store.list_recordings() {
                sources.push(recording_choice(path));
            }
        }
        let active = self
            .recorder
            .path()
            .map(Path::to_path_buf)
            .or_else(|| self.recorder.last_path().map(Path::to_path_buf));
        if let Some(path) = active.filter(|path| path.exists()) {
            let already_listed = sources.iter().any(|source| {
                matches!(&source.kind, ExportSourceKind::Recording(known) if known == &path)
            });
            if !already_listed {
                sources.push(recording_choice(path));
            }
        }
        sources
    }

    fn open_export_modal(&mut self) {
        let sources = self.export_sources();
        if sources.is_empty() {
            self.modal = Some(Modal::Error {
                title: "Export unavailable".to_string(),
                message: "No rolling history or recordings are available yet.".to_string(),
            });
            return;
        }
        let stamp = Local::now().format("%Y%m%d-%H%M%S").to_string();
        self.modal = Some(Modal::Export(ExportModal {
            sources,
            source: 0,
            window: 3,
            format: 0,
            stamp,
        }));
    }

    fn export_selected(&mut self, export: &ExportModal) {
        let Some(source) = export.sources.get(export.source).cloned() else {
            return;
        };
        let secs = export.window_secs();
        let format = export.format();
        let output = unique_path(export.output_path());
        let result = match format {
            ExportFormat::Pdf => self.export_pdf_source(&source, secs, &output),
            ExportFormat::Tsv => self.export_tsv_source(&source, secs, &output),
        };
        match result {
            Ok(rows) => {
                self.last_export = Some(output.clone());
                let unit = match format {
                    ExportFormat::Pdf => "samples",
                    ExportFormat::Tsv => "rows",
                };
                let name = output
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| output.display().to_string());
                self.status = format!("{name} ({} {unit})", rows);
                self.modal = None;
            }
            Err(err) => {
                self.modal = Some(Modal::Error {
                    title: "Export failed".to_string(),
                    message: err,
                });
            }
        }
    }

    fn load_export_samples(
        &mut self,
        source: &ExportSourceChoice,
        secs: Option<u64>,
    ) -> Result<Vec<Sample>, String> {
        match &source.kind {
            ExportSourceKind::Recent48h => self
                .store
                .as_mut()
                .ok_or_else(|| "rolling history unavailable".to_string())?
                .samples_since(secs),
            ExportSourceKind::Session => {
                let mut samples: Vec<Sample> = self.history.samples().cloned().collect();
                filter_export_samples(&mut samples, secs)?;
                Ok(samples)
            }
            ExportSourceKind::Recording(path) => {
                if self.recorder.is_recording() {
                    self.recorder
                        .flush()
                        .map_err(|e| format!("flush recording: {e}"))?;
                }
                let mut samples = raw::parse_file(path)?;
                filter_export_samples(&mut samples, secs)?;
                Ok(samples)
            }
        }
    }

    fn export_pdf_source(
        &mut self,
        source: &ExportSourceChoice,
        secs: Option<u64>,
        output: &Path,
    ) -> Result<usize, String> {
        let samples = self.load_export_samples(source, secs)?;
        let rails = self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.rails.clone())
            .unwrap_or_default();
        let system = self.sampler.system().clone();
        let input = ReportInput {
            samples: &samples,
            system: &system,
            rails: &rails,
            source_label: export_source_label(source, secs),
        };
        write_report(output, &input)?;
        Ok(samples.len())
    }

    fn export_tsv_source(
        &mut self,
        source: &ExportSourceChoice,
        secs: Option<u64>,
        output: &Path,
    ) -> Result<usize, String> {
        let label = export_source_label(source, secs);
        match &source.kind {
            ExportSourceKind::Recent48h => self
                .store
                .as_mut()
                .ok_or_else(|| "rolling history unavailable".to_string())?
                .export_tsv(output, secs, &label),
            ExportSourceKind::Session => {
                let samples = self.load_export_samples(source, secs)?;
                raw::write_samples_tsv(output, &samples, &label)
            }
            ExportSourceKind::Recording(path) => {
                if self.recorder.is_recording() {
                    self.recorder
                        .flush()
                        .map_err(|e| format!("flush recording: {e}"))?;
                }
                let latest = raw::last_timestamp(path)
                    .ok_or_else(|| format!("no samples in {}", path.display()))?;
                let first = raw::first_timestamp(path);
                let cutoff = secs
                    .map(|secs| {
                        latest
                            .checked_sub(Duration::from_secs(secs))
                            .unwrap_or(UNIX_EPOCH)
                    })
                    .or(first);
                let cutoff = match (cutoff, first) {
                    (Some(cutoff), Some(first)) => Some(cutoff.max(first)),
                    (cutoff, _) => cutoff,
                };
                raw::export_tsv_files(
                    std::slice::from_ref(path),
                    output,
                    cutoff,
                    Some(latest),
                    &label,
                )
            }
        }
    }

    pub fn export_on_exit(&mut self) -> Result<Option<PathBuf>, String> {
        let Some(path) = self.report_on_exit.clone() else {
            return Ok(None);
        };
        if self.history.is_empty() {
            return Err("no session samples for --report".to_string());
        }
        let source = ExportSourceChoice {
            label: "session history".to_string(),
            kind: ExportSourceKind::Session,
        };
        self.export_pdf_source(&source, None, &path)?;
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

fn recording_choice(path: PathBuf) -> ExportSourceChoice {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("recording");
    let live = name.ends_with(".part");
    let stored = name.starts_with("recording-");
    let stem = if stored {
        name.strip_prefix("recording-").unwrap_or(name)
    } else {
        name
    };
    let stem = stem
        .strip_suffix(".tsv.part")
        .or_else(|| stem.strip_suffix(".tsv"))
        .unwrap_or(stem);
    let display = if stored {
        NaiveDateTime::parse_from_str(stem, "%Y%m%d-%H%M%S")
            .map(|time| time.format("%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|_| stem.to_string())
    } else {
        stem.to_string()
    };
    let prefix = if stored { "Rec" } else { "Raw" };
    ExportSourceChoice {
        label: format!("{prefix} {display}{}", if live { " (partial)" } else { "" }),
        kind: ExportSourceKind::Recording(path),
    }
}

fn export_source_label(source: &ExportSourceChoice, secs: Option<u64>) -> String {
    let window = secs.map(window_label).unwrap_or_else(|| "All".to_string());
    format!("{} ({window})", source.label)
}

fn filter_export_samples(samples: &mut Vec<Sample>, secs: Option<u64>) -> Result<(), String> {
    if samples.is_empty() {
        return Err("no samples to export".to_string());
    }
    if let Some(secs) = secs {
        let latest = samples
            .iter()
            .map(|sample| sample.ts)
            .max()
            .expect("samples are non-empty");
        let cutoff = latest
            .checked_sub(Duration::from_secs(secs))
            .unwrap_or(UNIX_EPOCH);
        samples.retain(|sample| sample.ts >= cutoff);
    }
    if samples.is_empty() {
        return Err("no samples in selected range".to_string());
    }
    Ok(())
}

pub fn window_label(secs: u64) -> String {
    match secs {
        0 => "All".to_string(),
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s => format!("{}h", s / 3600),
    }
}

fn default_recording_path(dir: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let stamp = Local::now().format("%Y%m%d-%H%M%S");
    Ok(unique_path(dir.join(format!("recording-{stamp}.tsv.part"))))
}

fn finalize_recording(path: &Path) -> PathBuf {
    let Some(name) = path.to_str() else {
        return path.to_path_buf();
    };
    let Some(base) = name.strip_suffix(".part") else {
        return path.to_path_buf();
    };
    let target = PathBuf::from(base);
    if fs::rename(path, &target).is_ok() {
        target
    } else {
        path.to_path_buf()
    }
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
                history_dir: None,
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
    fn export_modal_opens_on_a_time_format_grid() {
        let mut app = app();
        app.tick();
        app.on_key(key(KeyCode::Char('e')));
        match &app.modal {
            Some(Modal::Export(export)) => {
                assert_eq!(export.sources.len(), 1);
                assert!(matches!(export.sources[0].kind, ExportSourceKind::Session));
                assert_eq!(export.window, 3);
                assert_eq!(export.format, 0);
            }
            _ => panic!("expected export modal"),
        }
        app.on_key(key(KeyCode::Down));
        app.on_key(key(KeyCode::Right));
        match &app.modal {
            Some(Modal::Export(export)) => {
                assert_eq!(export.window, 4);
                assert_eq!(export.format, 1);
            }
            _ => panic!("expected export modal"),
        }
    }

    #[test]
    fn export_actions_change_source_and_cell() {
        let mut app = app();
        app.tick();
        app.on_key(key(KeyCode::Char('e')));
        if let Some(Modal::Export(export)) = app.modal.as_mut() {
            export.sources.push(ExportSourceChoice {
                label: "test recording".to_string(),
                kind: ExportSourceKind::Recording(PathBuf::from("/tmp/test.tsv")),
            });
        }
        app.execute(Action::ExportSourceNext);
        match &app.modal {
            Some(Modal::Export(export)) => assert_eq!(export.source, 1),
            _ => panic!("expected export modal"),
        }
        app.execute(Action::ExportCell {
            window: 6,
            format: 1,
        });
        match &app.modal {
            Some(Modal::Export(export)) => {
                assert_eq!(export.window, 6);
                assert_eq!(export.format, 1);
            }
            _ => panic!("expected export modal"),
        }
    }

    #[test]
    fn long_history_window_loads_from_the_store() {
        let nonce = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_nanos();
        let root = std::env::temp_dir().join(format!("macpower-app-history-{nonce}"));
        let mut app = App::new(
            Sampler::demo(),
            AppOptions {
                interval_ms: 1000,
                history_secs: 60,
                window_secs: 60,
                mouse_enabled: true,
                ascii: false,
                raw_path: None,
                report_path: None,
                history_dir: Some(root.clone()),
            },
        );
        app.tick();
        app.window_secs = 172_800;
        app.refresh_history_cache();
        assert_eq!(
            app.history_cache.as_ref().map(|cache| cache.window_secs),
            Some(172_800)
        );
        assert!(!app.history_cache.as_ref().unwrap().samples.is_empty());

        app.window_secs = 60;
        app.refresh_history_cache();
        assert!(app.history_cache.is_none());
        drop(app);
        let _ = fs::remove_dir_all(root);
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

    #[test]
    fn mouse_move_does_not_request_redraw() {
        let mut app = app();
        app.needs_redraw = false;
        app.on_mouse(MouseEvent {
            kind: MouseEventKind::Moved,
            column: 3,
            row: 1,
            modifiers: KeyModifiers::NONE,
        });
        assert!(!app.needs_redraw);
    }
}
