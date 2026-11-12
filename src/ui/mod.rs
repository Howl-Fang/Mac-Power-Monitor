pub mod cpu;
pub mod help;
pub mod history;
pub mod overview;
pub mod power;
pub mod theme;
pub mod widgets;

use chrono::Local;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{Action, App, ExportFormat, ExportModal, Modal, Tab, EXPORT_WINDOWS};
use crate::model::{fmt_battery_power, fmt_pct, fmt_power};
use theme::Theme;

pub fn render(frame: &mut Frame, app: &mut App, theme: &Theme) {
    app.hit_zones.clear();
    let area = frame.area();
    if area.width < 80 || area.height < 24 {
        let msg = format!(
            "Terminal too small: {}x{}. macpower needs at least 80x24.",
            area.width, area.height
        );
        frame.render_widget(
            Paragraph::new(msg)
                .style(theme.error())
                .alignment(Alignment::Center)
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }

    let [header, tabs, content, status, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    app.content_rect = content;

    render_header(frame, header, app, theme);
    render_tabs(frame, tabs, app, theme);
    match app.tab {
        Tab::Overview => overview::render(frame, content, app, theme),
        Tab::Cpu => cpu::render(frame, content, app, theme),
        Tab::Power => power::render(frame, content, app, theme),
        Tab::History => history::render(frame, content, app, theme),
        Tab::Help => help::render(frame, content, app, theme),
    }
    render_status(frame, status, app, theme);
    render_footer(frame, footer, app, theme);

    if app.modal.is_some() {
        render_modal(frame, area, app, theme);
    }
}

fn render_header(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let time = Local::now().format("%H:%M:%S").to_string();
    let system = app.sampler.system();
    let mut left = vec![
        Span::styled("MacPower", theme.title()),
        Span::raw(" "),
        Span::styled(time, theme.dim_text()),
        Span::raw("  "),
        Span::styled(
            format!("{} / macOS {}", system.chip, system.os_version),
            theme.dim_text(),
        ),
    ];

    let right_text = match &app.snapshot {
        Some(snapshot) => {
            let b = &snapshot.battery;
            if b.present {
                let pct = fmt_pct(b.pct);
                let power = fmt_battery_power(&snapshot.sample);
                if b.external_connected {
                    if b.is_charging {
                        format!("AC {} charging {}", pct, power)
                    } else {
                        format!("AC {} {}", pct, power)
                    }
                } else {
                    format!(
                        "Batt {} {}  {} left",
                        pct,
                        power,
                        crate::model::fmt_duration_min(b.time_remaining_min)
                    )
                }
            } else {
                format!(
                    "AC {} {}",
                    fmt_pct(None),
                    fmt_power(snapshot.sample.system_w)
                )
            }
        }
        None => "warming up".to_string(),
    };

    let warn = if app.warnings.is_empty() {
        String::new()
    } else {
        format!("  !{}", app.warnings.len())
    };
    let right_len = (right_text.len() + warn.len()) as u16 + 2;
    let [left_area, right_area] = Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(right_len.min(area.width)),
    ])
    .areas(area);
    left.push(Span::raw(""));
    frame.render_widget(
        Paragraph::new(Line::from(left)).style(theme.text()),
        left_area,
    );
    let mut right_spans = vec![Span::styled(right_text, theme.text())];
    if !warn.is_empty() {
        right_spans.push(Span::styled(warn, theme.warn()));
    }
    frame.render_widget(
        Paragraph::new(Line::from(right_spans))
            .style(theme.text())
            .alignment(Alignment::Right),
        right_area,
    );
}

fn render_tabs(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    let widths = [
        Constraint::Length(12),
        Constraint::Length(8),
        Constraint::Length(9),
        Constraint::Length(11),
        Constraint::Length(8),
        Constraint::Min(0),
    ];
    let [a, b, c, d, e, _] = Layout::horizontal(widths).areas(area);
    let rects = [a, b, c, d, e];
    for (i, tab) in Tab::ALL.iter().enumerate() {
        let selected = app.tab == *tab;
        let label = format!(" {} ", tab.title());
        let style = if selected {
            theme.selected()
        } else {
            theme.dim_text()
        };
        let rect = rects[i];
        frame.render_widget(
            Paragraph::new(label)
                .style(style)
                .alignment(Alignment::Center),
            rect,
        );
        app.hit_zones.push((rect, Action::SwitchTab(*tab)));
    }
}

fn render_status(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let mut spans = Vec::new();
    if app.recorder.is_recording() {
        spans.push(Span::styled("● REC ", theme.error()));
        if let Some(path) = app.recorder.path() {
            spans.push(Span::styled(
                format!("{} ", path.display()),
                theme.dim_text(),
            ));
        }
    } else if app.paused {
        spans.push(Span::styled("PAUSED ", theme.warn()));
    }
    spans.push(Span::styled(app.status.clone(), theme.dim_text()));

    let right = format!(
        "{}ms | win {} | mouse {}",
        app.interval_ms,
        crate::app::window_label(app.window_secs),
        if app.mouse_enabled { "on" } else { "off" }
    );
    let right_len = right.len() as u16 + 1;
    let [left_area, right_area] = Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(right_len.min(area.width)),
    ])
    .areas(area);
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(theme.text()),
        left_area,
    );
    frame.render_widget(
        Paragraph::new(right)
            .style(theme.dim_text())
            .alignment(Alignment::Right),
        right_area,
    );
}

fn render_footer(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    let record_label = if app.recorder.is_recording() {
        "[R] Stop"
    } else {
        "[R] Record"
    };
    let pause_label = if app.paused {
        "[P] Resume"
    } else {
        "[P] Pause"
    };
    let buttons = [
        (record_label.to_string(), Action::ToggleRecord),
        ("[E] Export".to_string(), Action::ExportPdf),
        (pause_label.to_string(), Action::TogglePause),
        ("[?] Help".to_string(), Action::ShowHelp),
    ];
    let widths = [
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(10),
        Constraint::Min(0),
    ];
    let [a, b, c, d, rest] = Layout::horizontal(widths).areas(area);
    let rects = [a, b, c, d];
    for (i, (label, action)) in buttons.iter().enumerate() {
        let rect = rects[i];
        frame.render_widget(
            Paragraph::new(label.clone())
                .style(theme.text())
                .alignment(Alignment::Center),
            rect,
        );
        app.hit_zones.push((rect, action.clone()));
    }
    let hints = "q quit  m mouse  +/- int  t win";
    frame.render_widget(
        Paragraph::new(hints)
            .style(theme.dim_text())
            .alignment(Alignment::Right),
        rest,
    );
}

fn render_export_modal(
    frame: &mut Frame,
    area: Rect,
    export: &ExportModal,
    app: &mut App,
    theme: &Theme,
) {
    let height = 16.min(area.height.saturating_sub(2));
    let width = area.width.saturating_sub(8).min(78);
    let rect = centered(area, width, height);
    frame.render_widget(Clear, rect);
    let block = Block::bordered()
        .title(Line::from(vec![Span::styled(
            "Export History",
            theme.title(),
        )]))
        .border_style(theme.border_style());
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    if inner.height < 14 {
        return;
    }

    let [source_area, gap1, header_area, grid_area, gap2, output_area, footer_area] =
        Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(EXPORT_WINDOWS.len() as u16),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(inner);
    let _ = (gap1, gap2);

    render_export_source_row(frame, source_area, export, app, theme);

    let widths = [
        Constraint::Length(12),
        Constraint::Length(16),
        Constraint::Length(16),
        Constraint::Min(0),
    ];
    let header_cols = Layout::horizontal(widths).split(header_area);
    frame.render_widget(
        Paragraph::new("Time scale").style(theme.dim_text()),
        header_cols[0],
    );
    for (i, export_format) in ExportFormat::ALL.iter().enumerate() {
        let style = if export.format == i {
            theme.title()
        } else {
            theme.dim_text()
        };
        frame.render_widget(
            Paragraph::new(export_format.label())
                .style(style)
                .alignment(Alignment::Center),
            header_cols[i + 1],
        );
    }

    let rows = Layout::vertical(vec![Constraint::Length(1); EXPORT_WINDOWS.len()]).split(grid_area);
    for (row_idx, (label, _)) in EXPORT_WINDOWS.iter().enumerate() {
        let cols = Layout::horizontal(widths).split(rows[row_idx]);
        let time_style = if export.window == row_idx {
            theme.title()
        } else {
            theme.text()
        };
        frame.render_widget(Paragraph::new(*label).style(time_style), cols[0]);
        for (col_idx, _) in ExportFormat::ALL.iter().enumerate() {
            let cell = cols[col_idx + 1];
            let selected = export.window == row_idx && export.format == col_idx;
            let (text, style) = if selected {
                ("[x]", theme.selected())
            } else {
                ("[ ]", theme.dim_text())
            };
            frame.render_widget(
                Paragraph::new(Span::styled(text, style)).alignment(Alignment::Center),
                cell,
            );
            app.hit_zones.push((
                cell,
                Action::ExportCell {
                    window: row_idx,
                    format: col_idx,
                },
            ));
        }
    }

    let output = format!("Output: {}", export.output_path().display());
    frame.render_widget(
        Paragraph::new(truncate(&output, inner.width as usize)).style(theme.dim_text()),
        output_area,
    );

    let [hint_area, button_area, esc_area] = Layout::horizontal([
        Constraint::Min(0),
        Constraint::Length(12),
        Constraint::Length(12),
    ])
    .areas(footer_area);
    frame.render_widget(
        Paragraph::new("Tab source   arrows cell").style(theme.dim_text()),
        hint_area,
    );
    frame.render_widget(
        Paragraph::new("[ Export ]")
            .style(theme.selected())
            .alignment(Alignment::Center),
        button_area,
    );
    app.hit_zones.push((button_area, Action::ExportRun));
    frame.render_widget(
        Paragraph::new("Esc cancel")
            .style(theme.dim_text())
            .alignment(Alignment::Right),
        esc_area,
    );
    app.hit_zones.push((esc_area, Action::ModalCancel));
}

fn render_export_source_row(
    frame: &mut Frame,
    area: Rect,
    export: &ExportModal,
    app: &mut App,
    theme: &Theme,
) {
    let label = export
        .sources
        .get(export.source)
        .map(|source| source.label.as_str())
        .unwrap_or("n/a");
    let count = export.sources.len().max(1);
    let prefix = "Source:  ";
    let suffix = format!("  ({}/{})", export.source + 1, count);
    let reserved = prefix.len() + 4 + suffix.len();
    let available = (area.width as usize).saturating_sub(reserved);
    let label = truncate(label, available);
    let arrow_style = if export.sources.len() > 1 {
        theme.title()
    } else {
        theme.dim_text()
    };
    let line = Line::from(vec![
        Span::styled(prefix, theme.dim_text()),
        Span::styled("< ", arrow_style),
        Span::styled(label.clone(), theme.text()),
        Span::styled(" >", arrow_style),
        Span::styled(suffix, theme.dim_text()),
    ]);
    frame.render_widget(Paragraph::new(line), area);

    if export.sources.len() > 1 {
        let left_x = area.x + prefix.len() as u16;
        let right_x = left_x + 2 + label.chars().count() as u16 + 1;
        app.hit_zones
            .push((Rect::new(left_x, area.y, 1, 1), Action::ExportSourcePrev));
        app.hit_zones
            .push((Rect::new(right_x, area.y, 1, 1), Action::ExportSourceNext));
    }
}

fn render_modal(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    let Some(modal) = app.modal.clone() else {
        return;
    };
    match modal {
        Modal::Export(export) => render_export_modal(frame, area, &export, app, theme),
        Modal::Error { title, message } => {
            let width = area.width.saturating_sub(10).min(74);
            let height = 10.min(area.height.saturating_sub(2));
            let rect = centered(area, width, height);
            frame.render_widget(Clear, rect);
            let block = Block::bordered()
                .title(Line::from(vec![Span::styled(title.clone(), theme.error())]))
                .border_style(theme.error());
            let inner = block.inner(rect);
            frame.render_widget(block, rect);
            let text = format!("{message}\n\nPress Enter or Esc to close.");
            frame.render_widget(
                Paragraph::new(text)
                    .style(theme.text())
                    .wrap(Wrap { trim: true }),
                inner,
            );
            app.hit_zones.push((inner, Action::ModalCancel));
        }
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width.min(area.width), height.min(area.height))
}

fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('~');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{
        KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    };
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Position;
    use ratatui::Terminal;

    use crate::app::{AppOptions, Tab};
    use crate::collectors::Sampler;

    fn test_app() -> App {
        App::new(
            Sampler::demo(),
            AppOptions {
                interval_ms: 1000,
                history_secs: 3600,
                window_secs: 60,
                mouse_enabled: true,
                ascii: false,
                raw_path: None,
                report_path: None,
                history_dir: None,
            },
        )
    }

    fn draw(app: &mut App, width: u16, height: u16, theme: &Theme) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| render(frame, app, theme))
            .expect("draw");
        buffer_text(terminal.backend().buffer())
    }

    fn buffer_text(buffer: &Buffer) -> String {
        let area = *buffer.area();
        (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| {
                        buffer
                            .cell(Position::new(x, y))
                            .map(|cell| cell.symbol())
                            .unwrap_or(" ")
                    })
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn click(app: &mut App, rect: Rect) {
        app.on_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: rect.x + rect.width / 2,
            row: rect.y + rect.height / 2,
            modifiers: KeyModifiers::NONE,
        });
    }

    #[test]
    fn renders_all_tabs_at_80x24() {
        let theme = Theme::new(false);
        let mut app = test_app();
        app.tick();
        for tab in Tab::ALL {
            app.tab = tab;
            let text = draw(&mut app, 80, 24, &theme);
            assert!(text.contains("MacPower"), "missing header on {tab:?}");
            assert!(text.contains(tab.title()), "missing title on {tab:?}");
        }
        assert!(!app.hit_zones.is_empty());
    }

    #[test]
    fn mouse_hit_zones_navigate_and_change_window() {
        let theme = Theme::new(false);
        let mut app = test_app();
        app.tick();
        draw(&mut app, 80, 24, &theme);

        let (rect, _) = app
            .hit_zones
            .iter()
            .find(|(_, action)| matches!(action, Action::SwitchTab(Tab::Cpu)))
            .cloned()
            .expect("CPU tab hit zone");
        click(&mut app, rect);
        assert_eq!(app.tab, Tab::Cpu);

        app.tab = Tab::History;
        draw(&mut app, 80, 24, &theme);
        let (rect, _) = app
            .hit_zones
            .iter()
            .find(|(_, action)| matches!(action, Action::SetWindow(300)))
            .cloned()
            .expect("5m window hit zone");
        click(&mut app, rect);
        assert_eq!(app.window_secs, 300);
    }

    #[test]
    fn renders_export_modal_and_small_terminal_message() {
        let theme = Theme::new(false);
        let mut app = test_app();
        app.tick();
        app.on_key(KeyEvent {
            code: KeyCode::Char('e'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        let text = draw(&mut app, 80, 24, &theme);
        assert!(text.contains("Export History"));
        assert!(text.contains("This session"));
        assert!(text.contains("PDF"));
        assert!(text.contains("TSV"));
        assert!(text.contains("48 hours"));
        assert!(text.contains("Output:"));
        assert!(app
            .hit_zones
            .iter()
            .any(|(_, action)| matches!(action, Action::ExportCell { .. })));
        assert!(app
            .hit_zones
            .iter()
            .any(|(_, action)| matches!(action, Action::ExportRun)));
        let (cancel_rect, _) = app
            .hit_zones
            .iter()
            .find(|(_, action)| matches!(action, Action::ModalCancel))
            .cloned()
            .expect("cancel hit zone");
        click(&mut app, cancel_rect);
        assert!(app.modal.is_none());

        let text = draw(&mut app, 79, 23, &theme);
        assert!(text.contains("Terminal too small"));
    }

    #[test]
    fn export_modal_mouse_selects_a_grid_cell() {
        let theme = Theme::new(false);
        let mut app = test_app();
        app.tick();
        app.on_key(KeyEvent {
            code: KeyCode::Char('e'),
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        });
        draw(&mut app, 80, 24, &theme);
        let (rect, _) = app
            .hit_zones
            .iter()
            .find(|(_, action)| {
                matches!(
                    action,
                    Action::ExportCell {
                        window: 7,
                        format: 1
                    }
                )
            })
            .cloned()
            .expect("All/TSV cell");
        click(&mut app, rect);
        match app.modal {
            Some(Modal::Export(export)) => {
                assert_eq!(export.window, 7);
                assert_eq!(export.format, 1);
                assert_eq!(export.format().label(), "TSV");
            }
            _ => panic!("expected export modal"),
        }
    }

    #[test]
    fn overview_expands_with_history_when_tall() {
        let theme = Theme::new(false);
        let mut app = test_app();
        app.tick();
        app.tick();
        app.tab = Tab::Overview;

        let compact = draw(&mut app, 80, 24, &theme);
        assert!(!compact.contains("POWER HISTORY"));

        let minimum = draw(&mut app, 80, 29, &theme);
        assert!(minimum.contains("POWER HISTORY"));

        let text = draw(&mut app, 100, 34, &theme);
        assert!(text.contains("POWER HISTORY"));
        assert!(text.contains("BAT% history"));
        assert!(text.contains("COMPONENTS"));
        assert!(text.contains("CPU CORES"));
    }

    #[test]
    fn overview_component_meters_align() {
        let theme = Theme::new(false);
        let mut app = test_app();
        app.tick();
        app.tab = Tab::Overview;
        let text = draw(&mut app, 80, 24, &theme);
        let labels = ["CPU", "GPU", "ANE", "DRAM", "DISPLAY", "OTHER"];
        let mut brackets = Vec::new();
        let mut cpu_line = None;
        for line in text.lines() {
            let content = line.trim_start_matches('│');
            let Some(label) = labels.iter().find(|label| content.starts_with(**label)) else {
                continue;
            };
            if let Some(pos) = line.find('[') {
                brackets.push(pos);
            }
            if *label == "CPU" {
                cpu_line = Some(line);
            }
        }
        assert_eq!(brackets.len(), 6, "component meters missing");
        assert!(
            brackets.iter().all(|pos| *pos == brackets[0]),
            "meters not aligned: {brackets:?}"
        );
        assert!(cpu_line.unwrap().contains("(E"));
    }

    #[test]
    fn cpu_tab_adds_history_when_all_cores_fit() {
        let theme = Theme::new(false);
        let mut app = test_app();
        app.tick();
        app.tick();
        app.tab = Tab::Cpu;

        // Eight cores leave enough room for a chart even at 80x24.
        let text = draw(&mut app, 80, 24, &theme);
        assert!(text.contains("CPU POWER HISTORY"));
        assert!(text.contains("PER-CORE POWER"));

        // Many-core chips keep every core row and drop the chart instead.
        if let Some(snapshot) = app.snapshot.as_mut() {
            snapshot.sample.core_e_w = vec![0.01; 10];
            snapshot.sample.core_p_w = vec![0.01; 8];
        }
        let compact = draw(&mut app, 80, 24, &theme);
        assert!(!compact.contains("CPU POWER HISTORY"));
        assert!(compact.contains("E9"));
        assert!(compact.contains("P0"));

        let tall = draw(&mut app, 100, 40, &theme);
        assert!(tall.contains("CPU POWER HISTORY"));
    }

    #[test]
    fn renders_ascii_and_mono_modes() {
        let theme = Theme::new(true);
        let mut app = test_app();
        app.tick();
        app.ascii = true;
        app.tab = Tab::History;
        let text = draw(&mut app, 80, 24, &theme);
        assert!(text.contains("Power history"));
    }
}
