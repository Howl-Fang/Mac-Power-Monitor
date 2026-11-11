use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::model::{fmt_power, fmt_power_val, RailSource};
use crate::ui::theme::Theme;

pub fn render(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    let block = Block::bordered()
        .title(Line::from(vec![Span::styled("POWER RAILS", theme.title())]))
        .border_style(theme.border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 4 {
        return;
    }
    let Some(snapshot) = app.snapshot.as_ref() else {
        frame.render_widget(
            Paragraph::new("Waiting for the first sample...").style(theme.dim_text()),
            inner,
        );
        return;
    };

    let adapter = match (
        snapshot.battery.adapter_name.as_deref(),
        snapshot.battery.adapter_watts,
    ) {
        (Some(name), Some(w)) => format!("{name} {w:.0} W"),
        (Some(name), None) => name.to_string(),
        (None, Some(w)) => format!("{w:.0} W"),
        (None, None) => "n/a".to_string(),
    };
    let summary = format!(
        "PSTR {}   DC-IN {}   BAT {}   adapter {}   FW in {}",
        fmt_power(snapshot.smc.pstr_w),
        fmt_power(snapshot.smc.pdtr_w),
        fmt_power(snapshot.battery.power_w),
        adapter,
        fmt_power(snapshot.battery.fw_system_power_in_w)
    );
    let [summary_area, header_area, rows_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(inner);
    frame.render_widget(
        Paragraph::new(truncate(&summary, summary_area.width as usize)).style(theme.dim_text()),
        summary_area,
    );

    let header = format!(
        "{:<28} {:>10} {:<9} {:<9} {}",
        "RAIL", "POWER", "RAW UNIT", "SOURCE", "GROUP"
    );
    frame.render_widget(
        Paragraph::new(truncate(&header, header_area.width as usize)).style(theme.title()),
        header_area,
    );

    let rails = &snapshot.rails;
    if rails.is_empty() {
        frame.render_widget(
            Paragraph::new("No IOReport/SMC rails available.").style(theme.warn()),
            rows_area,
        );
        return;
    }

    let visible = rows_area.height as usize;
    let max_scroll = rails.len().saturating_sub(visible);
    app.power_scroll = app.power_scroll.min(max_scroll);
    let start = app.power_scroll;
    let end = (start + visible).min(rails.len());
    let mut lines = Vec::with_capacity(visible);
    for rail in &rails[start..end] {
        let color = match rail.source {
            RailSource::IoReport => theme.accent,
            RailSource::Smc => theme.yellow,
            RailSource::IoKit => theme.green,
            RailSource::Firmware => theme.dim,
        };
        let line = format!(
            "{:<28} {:>10} {:<9} {:<9} {}",
            truncate(&rail.name, 28),
            fmt_power_val(rail.w),
            truncate(&rail.unit, 9),
            rail.source.label(),
            truncate(&rail.group, rows_area.width.saturating_sub(60) as usize)
        );
        lines.push(Line::from(Span::styled(line, Style::default().fg(color))));
    }
    frame.render_widget(Paragraph::new(lines), rows_area);
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
