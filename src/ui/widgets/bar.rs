use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::model::fmt_power_val;
use crate::ui::theme::Theme;

pub fn render_bar_row(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    value: f32,
    max: f32,
    color: Color,
    theme: &Theme,
) {
    let text = fmt_power_val(value);
    let ratio = (value / max.max(0.0001)).clamp(0.0, 1.0) as f64;
    render_bar_row_text(frame, area, label, ratio, color, &text, theme);
}

pub fn render_bar_row_text(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    ratio: f64,
    color: Color,
    text: &str,
    theme: &Theme,
) {
    render_meter(frame, area, label, text, ratio, color, theme);
}

pub fn render_battery_gauge(
    frame: &mut Frame,
    area: Rect,
    pct: Option<f32>,
    label: String,
    theme: &Theme,
) {
    let pct = pct.unwrap_or(0.0).clamp(0.0, 100.0);
    let color = if pct <= 15.0 {
        theme.red
    } else if pct <= 35.0 {
        theme.yellow
    } else {
        theme.green
    };
    render_meter(
        frame,
        area,
        "BAT",
        &label,
        (pct / 100.0) as f64,
        color,
        theme,
    );
}

/// A bracket meter without an external label, used for compact rows.
pub fn render_meter_bar(
    frame: &mut Frame,
    area: Rect,
    ratio: f64,
    color: Color,
    text: &str,
    theme: &Theme,
) {
    render_meter(frame, area, "", text, ratio, color, theme);
}

/// Draw `LABEL[||||  value]`.
///
/// Filled columns use `|`; empty columns are blank. The value is right
/// aligned inside the bar, and each of its characters is colored according
/// to whether its column falls inside the filled part of the meter.
fn render_meter(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    text: &str,
    ratio: f64,
    color: Color,
    theme: &Theme,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let min_bar_width = 8;
    let max_label = area.width.saturating_sub(min_bar_width + 1) as usize;
    let label = truncate(label, max_label);
    let label_width = label.chars().count() as u16;
    let has_label = !label.is_empty();
    let bar_width = area
        .width
        .saturating_sub(if has_label { label_width + 1 } else { 0 });
    if bar_width < 4 {
        let fallback = if text.is_empty() {
            label
        } else if has_label {
            format!("{label} {text}")
        } else {
            text.to_string()
        };
        frame.render_widget(
            Paragraph::new(truncate(&fallback, area.width as usize))
                .style(Style::default().fg(color)),
            area,
        );
        return;
    }

    if has_label {
        let label_area = Rect::new(area.x, area.y, label_width, 1);
        frame.render_widget(
            Paragraph::new(label).style(Style::default().fg(color)),
            label_area,
        );
    }
    let bar_x = area.x + if has_label { label_width + 1 } else { 0 };
    let bar_area = Rect::new(bar_x, area.y, bar_width, 1);
    let inner_width = (bar_width - 2) as usize;
    let fill = (ratio.clamp(0.0, 1.0) * inner_width as f64).round() as usize;
    frame.render_widget(
        Paragraph::new(meter_line(inner_width, fill, text, color, theme)),
        bar_area,
    );
}

fn meter_line(
    inner_width: usize,
    fill: usize,
    text: &str,
    color: Color,
    theme: &Theme,
) -> Line<'static> {
    let text_chars: Vec<char> = text.chars().collect();
    let text_width = text_chars.len().min(inner_width);
    let text_start = inner_width - text_width;
    let inside_style = if theme.no_color {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(color)
    };
    let mut spans = Vec::with_capacity(inner_width + 2);
    spans.push(Span::styled("[", inside_style));
    for col in 0..inner_width {
        let (ch, style) = match col.checked_sub(text_start) {
            Some(i) => {
                let style = if col < fill {
                    inside_style
                } else {
                    theme.text()
                };
                (text_chars[i], style)
            }
            None if col < fill => ('|', inside_style),
            None => (' ', Style::default()),
        };
        spans.push(Span::styled(ch.to_string(), style));
    }
    spans.push(Span::styled("]", inside_style));
    Line::from(spans)
}

fn truncate(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('~');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_fills_and_aligns_text_right() {
        let theme = Theme::new(false);
        let line = meter_line(10, 5, "50%", theme.blue, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "[|||||  50%]");
    }

    #[test]
    fn text_color_tracks_the_fill_boundary() {
        let theme = Theme::new(false);
        let line = meter_line(10, 9, "50%", theme.blue, &theme);
        let chars: Vec<&Span> = line.spans.iter().skip(1).take(10).collect();
        // "50%" starts at column 7; columns 7 and 8 are filled, 9 is empty.
        assert_eq!(chars[7].content.as_ref(), "5");
        assert_eq!(chars[7].style.fg, Some(theme.blue));
        assert_eq!(chars[8].content.as_ref(), "0");
        assert_eq!(chars[8].style.fg, Some(theme.blue));
        assert_eq!(chars[9].content.as_ref(), "%");
        assert_eq!(chars[9].style.fg, Some(theme.fg));
    }

    #[test]
    fn mono_meter_separates_inside_text_with_bold() {
        let theme = Theme::new(true);
        let line = meter_line(10, 9, "50%", theme.blue, &theme);
        let chars: Vec<&Span> = line.spans.iter().skip(1).take(10).collect();
        assert!(chars[7].style.add_modifier.contains(Modifier::BOLD));
        assert!(chars[8].style.add_modifier.contains(Modifier::BOLD));
        assert!(!chars[9].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn long_text_is_truncated_to_the_inner_width() {
        let theme = Theme::new(false);
        let line = meter_line(5, 0, "123456789", theme.green, &theme);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "[12345]");
    }

    #[test]
    fn long_labels_leave_room_for_the_bar() {
        assert_eq!(truncate("a-very-long-label", 6), "a-ver~");
    }
}
