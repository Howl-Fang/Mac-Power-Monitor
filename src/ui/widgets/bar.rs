use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Gauge;
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
    let ratio = (value / max.max(0.0001)).clamp(0.0, 1.0) as f64;
    let gauge = Gauge::default()
        .ratio(ratio)
        .label(format!("{label}  {}", fmt_power_val(value)))
        .gauge_style(Style::default().fg(color))
        .style(theme.text())
        .use_unicode(true);
    frame.render_widget(gauge, area);
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
    let gauge = Gauge::default()
        .ratio((pct / 100.0) as f64)
        .label(label)
        .gauge_style(Style::default().fg(color))
        .style(theme.text())
        .use_unicode(true);
    frame.render_widget(gauge, area);
}
