use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::canvas::{Canvas, Line as CanvasLine};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::ui::theme::Theme;

pub struct GraphSeries<'a> {
    pub label: &'a str,
    pub color: Color,
    pub data: Vec<(f64, f64)>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AxisUnit {
    #[default]
    Power,
    Percent,
}

#[derive(Clone, Copy, Default)]
pub struct GraphOptions {
    pub y_min: Option<f64>,
    pub y_max: Option<f64>,
    pub unit: AxisUnit,
}

impl GraphOptions {
    pub fn power() -> Self {
        Self::default()
    }

    pub fn percent() -> Self {
        Self {
            y_min: Some(0.0),
            y_max: Some(100.0),
            unit: AxisUnit::Percent,
        }
    }
}

pub fn render_graph(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    series: &[GraphSeries<'_>],
    options: GraphOptions,
    theme: &Theme,
    ascii: bool,
) {
    if area.width < 4 || area.height < 3 {
        return;
    }
    let block = Block::bordered()
        .title(Span::styled(title.to_string(), theme.title()))
        .border_style(theme.border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let (ymin, ymax) = range(series, options.y_min, options.y_max);
    if series.iter().all(|s| s.data.is_empty()) {
        frame.render_widget(
            Paragraph::new(Span::styled("waiting for samples...", theme.dim_text())),
            inner,
        );
        return;
    }

    let labels = axis_labels(ymin, ymax, options.unit);
    let label_width = labels
        .iter()
        .map(|label| label.chars().count())
        .max()
        .unwrap_or(0) as u16;
    let mut plot_area = inner;
    if inner.height >= 3 && inner.width > label_width + 6 {
        let [axis_area, plot] =
            Layout::horizontal([Constraint::Length(label_width), Constraint::Min(0)]).areas(inner);
        render_axis_labels(frame, axis_area, &labels, theme);
        plot_area = plot;
    }

    if ascii {
        render_ascii(frame, plot_area, series, ymin, ymax, theme);
    } else {
        let canvas = Canvas::default()
            .marker(Marker::Braille)
            .background_color(Color::Reset)
            .x_bounds([0.0, 1.0])
            .y_bounds([ymin, ymax])
            .paint(|ctx| {
                for s in series {
                    if s.data.len() < 2 {
                        continue;
                    }
                    for pair in s.data.windows(2) {
                        let (x1, y1) = pair[0];
                        let (x2, y2) = pair[1];
                        ctx.draw(&CanvasLine::new(x1, y1, x2, y2, s.color));
                    }
                }
            });
        frame.render_widget(canvas, plot_area);
    }
}

fn axis_labels(ymin: f64, ymax: f64, unit: AxisUnit) -> [String; 3] {
    [
        axis_label(ymax, unit),
        axis_label((ymin + ymax) / 2.0, unit),
        axis_label(ymin, unit),
    ]
}

fn axis_label(value: f64, unit: AxisUnit) -> String {
    if !value.is_finite() {
        return "n/a".to_string();
    }
    match unit {
        AxisUnit::Percent => format!("{value:.0}%"),
        AxisUnit::Power if value.abs() < 1.0 => format!("{:.0}mW", value * 1000.0),
        AxisUnit::Power if value.abs() < 10.0 => format!("{value:.1}W"),
        AxisUnit::Power => format!("{value:.0}W"),
    }
}

fn render_axis_labels(frame: &mut Frame, area: Rect, labels: &[String; 3], theme: &Theme) {
    let rows = [
        Rect::new(area.x, area.y, area.width, 1),
        Rect::new(area.x, area.y + area.height / 2, area.width, 1),
        Rect::new(area.x, area.y + area.height - 1, area.width, 1),
    ];
    for (row, label) in rows.into_iter().zip(labels) {
        frame.render_widget(
            Paragraph::new(label.as_str())
                .style(theme.dim_text())
                .alignment(Alignment::Right),
            row,
        );
    }
}

fn range(series: &[GraphSeries<'_>], y_min: Option<f64>, y_max: Option<f64>) -> (f64, f64) {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for s in series {
        for (_, y) in &s.data {
            if y.is_finite() {
                lo = lo.min(*y);
                hi = hi.max(*y);
            }
        }
    }
    let mut lo = y_min.unwrap_or(if lo.is_finite() { lo.min(0.0) } else { 0.0 });
    let mut hi = y_max.unwrap_or(if hi.is_finite() { hi } else { 1.0 });
    if !lo.is_finite() || !hi.is_finite() {
        return (0.0, 1.0);
    }
    if hi <= lo {
        hi = lo + 1.0;
    }
    let pad = (hi - lo) * 0.08;
    if y_max.is_none() {
        hi += pad;
    }
    if y_min.is_none() && lo < 0.0 {
        lo -= pad;
    }
    (lo, hi)
}

fn render_ascii(
    frame: &mut Frame,
    area: Rect,
    series: &[GraphSeries<'_>],
    ymin: f64,
    ymax: f64,
    theme: &Theme,
) {
    let w = area.width as usize;
    let h = area.height as usize;
    if w == 0 || h == 0 {
        return;
    }
    let mut grid = vec![vec![' '; w]; h];
    let chars = ['*', '+', 'x', 'o', '#', '@', '%', '='];
    for (si, s) in series.iter().enumerate() {
        let ch = chars[si % chars.len()];
        let mut peaks: Vec<Option<f64>> = vec![None; w];
        for (x, y) in &s.data {
            if !x.is_finite() || !y.is_finite() {
                continue;
            }
            let col = ((*x * w as f64).floor() as usize).min(w - 1);
            peaks[col] = Some(peaks[col].map_or(*y, |v: f64| v.max(*y)));
        }
        for (col, value) in peaks.into_iter().enumerate() {
            let Some(value) = value else {
                continue;
            };
            let t = ((value - ymin) / (ymax - ymin)).clamp(0.0, 1.0);
            let row = ((1.0 - t) * (h.saturating_sub(1)) as f64).round() as usize;
            if let Some(line) = grid.get_mut(row) {
                line[col] = ch;
            }
        }
    }

    let mut lines = Vec::with_capacity(h);
    for row in grid {
        let text: String = row.into_iter().collect();
        lines.push(Line::from(Span::styled(
            text,
            Style::default().fg(theme.fg),
        )));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

pub fn legend_line(series: &[GraphSeries<'_>], theme: &Theme, ascii: bool) -> Line<'static> {
    let marker = if ascii { '*' } else { '■' };
    let mut spans = Vec::new();
    for (i, s) in series.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(
            format!("{marker} {}", s.label),
            Style::default().fg(s.color),
        ));
    }
    let _ = theme;
    Line::from(spans)
}

/// A compact one-line sparkline for the overview.
pub fn render_sparkline(
    frame: &mut Frame,
    area: Rect,
    values: &[u64],
    max: u64,
    color: Color,
    ascii: bool,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    if ascii {
        let text = ascii_sparkline(values, max, area.width as usize);
        frame.render_widget(
            Paragraph::new(Span::styled(text, Style::default().fg(color))),
            area,
        );
        return;
    }
    use ratatui::widgets::Sparkline;
    let spark = Sparkline::default()
        .data(values)
        .max(max.max(1))
        .style(Style::default().fg(color));
    frame.render_widget(spark, area);
}

fn ascii_sparkline(values: &[u64], max: u64, width: usize) -> String {
    let chars = [' ', '.', ':', '-', '=', '+', '*', '#'];
    let max = max.max(1);
    let mut out = vec![' '; width];
    let start = width.saturating_sub(values.len());
    for (i, value) in values.iter().rev().take(width).rev().enumerate() {
        let t = (*value as f64 / max as f64).clamp(0.0, 1.0);
        let idx = (t * (chars.len() - 1) as f64).round() as usize;
        out[start + i] = chars[idx.min(chars.len() - 1)];
    }
    out.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_power_axis_labels() {
        assert_eq!(axis_label(0.0, AxisUnit::Power), "0mW");
        assert_eq!(axis_label(0.5, AxisUnit::Power), "500mW");
        assert_eq!(axis_label(5.25, AxisUnit::Power), "5.2W");
        assert_eq!(axis_label(42.4, AxisUnit::Power), "42W");
        assert_eq!(axis_label(f64::NAN, AxisUnit::Power), "n/a");
    }

    #[test]
    fn formats_percent_axis_labels() {
        assert_eq!(axis_label(100.0, AxisUnit::Percent), "100%");
        assert_eq!(axis_label(50.0, AxisUnit::Percent), "50%");
    }

    #[test]
    fn ascii_sparkline_scales_and_pads() {
        assert_eq!(ascii_sparkline(&[0], 100, 1), " ");
        assert_eq!(ascii_sparkline(&[100], 100, 1), "#");
        assert_eq!(ascii_sparkline(&[50], 100, 1), "=");
        assert_eq!(ascii_sparkline(&[50, 100], 100, 3), " =#");
    }

    #[test]
    fn explicit_bounds_do_not_get_headroom() {
        let series = vec![GraphSeries {
            label: "battery",
            color: Color::Reset,
            data: vec![(0.0, 42.0), (1.0, 73.0)],
        }];
        assert_eq!(range(&series, Some(0.0), Some(100.0)), (0.0, 100.0));
    }
}
