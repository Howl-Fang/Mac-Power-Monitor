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

use crate::app::{Action, App, Modal, Tab};
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

fn render_modal(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    let Some(modal) = app.modal.clone() else {
        return;
    };
    match modal {
        Modal::Export(export) => {
            let height = (export.options.len() as u16 + 7).min(area.height.saturating_sub(2));
            let width = area.width.saturating_sub(8).min(78);
            let rect = centered(area, width, height);
            frame.render_widget(Clear, rect);
            let block = Block::bordered()
                .title(Line::from(vec![Span::styled(
                    "Export PDF Report",
                    theme.title(),
                )]))
                .border_style(theme.border_style());
            let inner = block.inner(rect);
            frame.render_widget(block, rect);

            let output = format!("Output: {}", export.output_path.display());
            let [out_area, options_area, hint_area] = Layout::vertical([
                Constraint::Length(2),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .areas(inner);
            frame.render_widget(
                Paragraph::new(truncate(&output, inner.width as usize))
                    .style(theme.dim_text())
                    .wrap(Wrap { trim: true }),
                out_area,
            );

            let option_rows = (options_area.y..).zip(export.options.iter().enumerate());
            for (y, (i, option)) in option_rows {
                if y >= options_area.y + options_area.height {
                    break;
                }
                let row = Rect::new(options_area.x, y, options_area.width, 1);
                let selected = i == export.selected;
                let marker = if selected { ">" } else { " " };
                let label = truncate(&format!("{marker} {}", option.label), row.width as usize);
                let style = if !option.enabled {
                    theme.dim_text()
                } else if selected {
                    theme.selected()
                } else {
                    theme.text()
                };
                frame.render_widget(Paragraph::new(label).style(style), row);
                app.hit_zones.push((row, Action::ModalSelect(i)));
            }
            frame.render_widget(
                Paragraph::new("Enter select   Esc cancel")
                    .style(theme.dim_text())
                    .alignment(Alignment::Right),
                hint_area,
            );
        }
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
        assert!(text.contains("Export PDF Report"));
        assert!(text.contains("Session history"));
        assert!(app
            .hit_zones
            .iter()
            .any(|(_, action)| matches!(action, Action::ModalSelect(_))));

        app.modal = None;
        let text = draw(&mut app, 79, 23, &theme);
        assert!(text.contains("Terminal too small"));
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

        let text = draw(&mut app, 100, 34, &theme);
        assert!(text.contains("POWER HISTORY"));
        assert!(text.contains("BAT% history"));
        assert!(text.contains("COMPONENTS"));
        assert!(text.contains("CPU CORES"));
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
