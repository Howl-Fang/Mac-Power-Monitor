use std::time::Duration;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::model::{
    fmt_battery_current, fmt_battery_power, fmt_duration_min, fmt_pct, fmt_power, fmt_temp,
    fmt_voltage,
};
use crate::ui::theme::Theme;
use crate::ui::widgets::bar::{render_bar_row, render_battery_gauge};
use crate::ui::widgets::graph;

pub fn render(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    let [system, battery, comps, cores] = Layout::vertical([
        Constraint::Length(5),
        Constraint::Length(4),
        Constraint::Length(8),
        Constraint::Length(3),
    ])
    .areas(area);

    let Some(snapshot) = app.snapshot.as_ref() else {
        frame.render_widget(
            Paragraph::new("Waiting for the first sample...")
                .style(theme.dim_text())
                .alignment(ratatui::layout::Alignment::Center),
            area,
        );
        return;
    };
    let sample = &snapshot.sample;

    let warming = app.history.len() <= 1 && sample.cpu_w.is_none();
    render_system(frame, system, app, theme);
    render_battery(frame, battery, app, theme);
    render_components(frame, comps, sample, theme, warming);
    render_cores(frame, cores, sample, theme, warming);
}

fn render_system(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = Block::bordered()
        .title(Line::from(vec![Span::styled(
            "SYSTEM POWER",
            theme.title(),
        )]))
        .border_style(theme.border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 3 {
        return;
    }
    let [line1, line2, line3] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);

    let Some(snapshot) = app.snapshot.as_ref() else {
        return;
    };
    let sample = &snapshot.sample;
    let source = match sample.source {
        crate::model::PowerSource::Battery => "battery",
        crate::model::PowerSource::Ac => "AC",
        crate::model::PowerSource::Unknown => "unknown",
    };
    let line = Line::from(vec![
        Span::styled(fmt_power(sample.system_w), theme.title()),
        Span::raw("  "),
        Span::styled(format!("source: {source}"), theme.dim_text()),
    ]);
    frame.render_widget(Paragraph::new(line).style(theme.text()), line1);

    let line = Line::from(vec![
        Span::styled("PSTR ", theme.dim_text()),
        Span::styled(fmt_power(sample.smc_pstr_w), theme.text()),
        Span::raw("   "),
        Span::styled("DC-IN ", theme.dim_text()),
        Span::styled(fmt_power(sample.dc_in_w), theme.text()),
        Span::raw("   "),
        Span::styled("BAT ", theme.dim_text()),
        Span::styled(fmt_battery_power(sample), theme.text()),
    ]);
    frame.render_widget(Paragraph::new(line).style(theme.text()), line2);

    let samples = app.history.window(Duration::from_secs(60));
    let take = line3.width as usize;
    let values: Vec<u64> = samples
        .iter()
        .rev()
        .take(take)
        .rev()
        .filter_map(|s| s.system_w)
        .map(|w| (w.max(0.0) * 1000.0) as u64)
        .collect();
    let max = values.iter().copied().max().unwrap_or(1);
    graph::render_sparkline(frame, line3, &values, max, theme.accent, app.ascii);
}

fn render_battery(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let block = Block::bordered()
        .title(Line::from(vec![Span::styled("BATTERY", theme.title())]))
        .border_style(theme.border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 2 {
        return;
    }
    let [line1, line2] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(inner);
    let Some(snapshot) = app.snapshot.as_ref() else {
        return;
    };
    let b = &snapshot.battery;
    if !b.present {
        frame.render_widget(
            Paragraph::new("No battery detected (desktop Mac or sensor unavailable).")
                .style(theme.dim_text()),
            line1,
        );
        frame.render_widget(
            Paragraph::new(format!(
                "DC-IN {}   system {}",
                fmt_power(snapshot.sample.dc_in_w),
                fmt_power(snapshot.sample.system_w)
            ))
            .style(theme.text()),
            line2,
        );
        return;
    }
    let label = if b.is_charging {
        format!(
            "{}  charging  {} to full",
            fmt_pct(b.pct),
            fmt_duration_min(b.time_to_full_min)
        )
    } else if b.external_connected {
        format!("{}  AC connected", fmt_pct(b.pct))
    } else {
        format!(
            "{}  {} remaining",
            fmt_pct(b.pct),
            fmt_duration_min(b.time_remaining_min)
        )
    };
    render_battery_gauge(frame, line1, b.pct, label, theme);

    let details = format!(
        "{}  {}  {}  {}  health {}  cycles {}",
        fmt_voltage(b.voltage_v),
        fmt_battery_current(&snapshot.sample),
        fmt_battery_power(&snapshot.sample),
        fmt_temp(b.temperature_c),
        fmt_pct(b.health_pct),
        b.cycle_count
            .map(|v| v.to_string())
            .unwrap_or_else(|| "n/a".into())
    );
    frame.render_widget(Paragraph::new(details).style(theme.text()), line2);
}

fn render_components(
    frame: &mut Frame,
    area: Rect,
    sample: &crate::model::Sample,
    theme: &Theme,
    warming: bool,
) {
    let block = Block::bordered()
        .title(Line::from(vec![Span::styled("COMPONENTS", theme.title())]))
        .border_style(theme.border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 1 {
        return;
    }
    if sample.cpu_w.is_none() && sample.gpu_w.is_none() && sample.dram_w.is_none() {
        let message = if warming {
            "Warming up IOReport counters (needs two samples)..."
        } else {
            "Component power unavailable (IOReport Energy Model not present)."
        };
        frame.render_widget(Paragraph::new(message).style(theme.warn()), inner);
        return;
    }
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(inner);

    let values = [
        sample.cpu_w.unwrap_or(0.0),
        sample.gpu_w.unwrap_or(0.0),
        sample.ane_w.unwrap_or(0.0),
        sample.dram_w.unwrap_or(0.0),
        sample.display_w.unwrap_or(0.0),
        sample.other_w.unwrap_or(0.0),
    ];
    let max = values.iter().copied().fold(0.0f32, f32::max).max(0.5);

    let cpu_label = match (sample.ecpu_w, sample.pcpu_w) {
        (Some(e), Some(p)) => format!("CPU (E {} / P {})", fmt_power(Some(e)), fmt_power(Some(p))),
        _ => "CPU".to_string(),
    };
    render_bar_row(
        frame,
        rows[0],
        &cpu_label,
        values[0],
        max,
        theme.green,
        theme,
    );
    render_bar_row(frame, rows[1], "GPU", values[1], max, theme.orange, theme);
    render_bar_row(frame, rows[2], "ANE", values[2], max, theme.purple, theme);
    render_bar_row(frame, rows[3], "DRAM", values[3], max, theme.red, theme);
    render_bar_row(
        frame,
        rows[4],
        "DISPLAY",
        values[4],
        max,
        theme.yellow,
        theme,
    );
    render_bar_row(frame, rows[5], "OTHER", values[5], max, theme.dim, theme);
}

fn render_cores(
    frame: &mut Frame,
    area: Rect,
    sample: &crate::model::Sample,
    theme: &Theme,
    warming: bool,
) {
    let block = Block::bordered()
        .title(Line::from(vec![Span::styled("CPU CORES", theme.title())]))
        .border_style(theme.border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 1 {
        return;
    }
    let mut spans = Vec::new();
    for (i, w) in sample.core_e_w.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(
            format!("E{i} "),
            Style::default().fg(theme.blue),
        ));
        spans.push(Span::styled(core_power(*w), theme.text()));
    }
    if !sample.core_e_w.is_empty() && !sample.core_p_w.is_empty() {
        spans.push(Span::styled(" | ", theme.dim_text()));
    }
    for (i, w) in sample.core_p_w.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(
            format!("P{i} "),
            Style::default().fg(theme.purple),
        ));
        spans.push(Span::styled(core_power(*w), theme.text()));
    }
    if spans.is_empty() {
        let message = if warming {
            "Warming up per-core counters..."
        } else {
            "Per-core channels unavailable on this Mac."
        };
        spans.push(Span::styled(message, theme.dim_text()));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).style(theme.text()), inner);
}

fn core_power(w: f32) -> String {
    if !w.is_finite() {
        return "n/a".to_string();
    }
    if w.abs() < 1.0 {
        format!("{:.0}mW", w * 1000.0)
    } else {
        format!("{:.2}W", w)
    }
}
