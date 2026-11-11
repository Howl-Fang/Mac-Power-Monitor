use ratatui::layout::Rect;
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

#[derive(Clone, Copy, Default)]
pub struct YRange {
    pub min: Option<f64>,
    pub max: Option<f64>,
}

pub fn render_graph(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    series: &[GraphSeries<'_>],
    y_range: YRange,
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

    let (ymin, ymax) = range(series, y_range.min, y_range.max);
    if series.iter().all(|s| s.data.is_empty()) {
        frame.render_widget(
            Paragraph::new(Span::styled("waiting for samples...", theme.dim_text())),
            inner,
        );
        return;
    }

    if ascii {
        render_ascii(frame, inner, series, ymin, ymax, theme);
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
        frame.render_widget(canvas, inner);
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
    hi += pad;
    if lo < 0.0 {
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
    theme: &Theme,
) {
    use ratatui::widgets::Sparkline;
    let spark = Sparkline::default()
        .data(values)
        .max(max.max(1))
        .style(Style::default().fg(color));
    frame.render_widget(spark, area);
    let _ = theme;
}
