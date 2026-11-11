use std::time::Duration;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Gauge, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::model::{fmt_power, Sample};
use crate::ui::theme::Theme;
use crate::ui::widgets::bar::render_bar_row;
use crate::ui::widgets::graph;

pub fn render(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    let [clusters, cores] =
        Layout::vertical([Constraint::Length(5), Constraint::Min(0)]).areas(area);
    let Some(snapshot) = app.snapshot.as_ref() else {
        frame.render_widget(
            Paragraph::new("Waiting for the first sample...")
                .style(theme.dim_text())
                .alignment(ratatui::layout::Alignment::Center),
            area,
        );
        return;
    };

    let block = Block::bordered()
        .title(Line::from(vec![Span::styled(
            "CPU CLUSTERS",
            theme.title(),
        )]))
        .border_style(theme.border_style());
    let inner = block.inner(clusters);
    frame.render_widget(block, clusters);
    let warming = app.history.len() <= 1 && snapshot.sample.cpu_w.is_none();
    if inner.height >= 3 {
        if warming {
            frame.render_widget(
                Paragraph::new("Warming up IOReport counters (needs two samples)...")
                    .style(theme.warn()),
                inner,
            );
        } else {
            let rows = Layout::vertical([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner);
            let cpu = snapshot.sample.cpu_w.unwrap_or(0.0);
            let e = snapshot.sample.ecpu_w.unwrap_or(0.0);
            let p = snapshot.sample.pcpu_w.unwrap_or(0.0);
            let max = cpu.max(e).max(p).max(0.5);
            render_bar_row(frame, rows[0], "CPU total", cpu, max, theme.green, theme);
            render_bar_row(frame, rows[1], "E-cluster", e, max, theme.blue, theme);
            render_bar_row(frame, rows[2], "P-cluster", p, max, theme.purple, theme);
        }
    }

    render_core_panel(frame, cores, &snapshot.sample, app, theme, warming);
}

fn render_core_panel(
    frame: &mut Frame,
    area: Rect,
    sample: &Sample,
    app: &App,
    theme: &Theme,
    warming: bool,
) {
    let block = Block::bordered()
        .title(Line::from(vec![Span::styled(
            "PER-CORE POWER",
            theme.title(),
        )]))
        .border_style(theme.border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 1 {
        return;
    }

    let cores: Vec<(char, usize, f32)> = sample
        .core_e_w
        .iter()
        .enumerate()
        .map(|(i, w)| ('E', i, *w))
        .chain(
            sample
                .core_p_w
                .iter()
                .enumerate()
                .map(|(i, w)| ('P', i, *w)),
        )
        .collect();
    if cores.is_empty() {
        let message = if warming {
            "Warming up per-core counters..."
        } else {
            "Per-core IOReport channels unavailable on this Mac."
        };
        frame.render_widget(Paragraph::new(message).style(theme.dim_text()), inner);
        return;
    }

    let max = cores
        .iter()
        .map(|(_, _, w)| *w)
        .fold(0.0f32, f32::max)
        .max(0.01);
    let history: Vec<&Sample> = app.history.window(Duration::from_secs(60));
    let count = (inner.height as usize).min(cores.len());
    let rows = Layout::vertical(vec![Constraint::Length(1); count]).split(inner);

    for (row_idx, (kind, idx, w)) in cores.iter().take(count).enumerate() {
        let row = rows[row_idx];
        let [label_area, bar_area, value_area, spark_area] = Layout::horizontal([
            Constraint::Length(6),
            Constraint::Length(14),
            Constraint::Length(10),
            Constraint::Min(0),
        ])
        .areas(row);

        let color = if *kind == 'E' {
            theme.blue
        } else {
            theme.purple
        };
        frame.render_widget(
            Paragraph::new(format!("{kind}{idx}")).style(Style::default().fg(color)),
            label_area,
        );
        let gauge = Gauge::default()
            .ratio((*w / max).clamp(0.0, 1.0) as f64)
            .gauge_style(Style::default().fg(color))
            .use_unicode(true);
        frame.render_widget(gauge, bar_area);
        frame.render_widget(
            Paragraph::new(fmt_power(Some(*w))).style(theme.text()),
            value_area,
        );

        let values: Vec<u64> = history
            .iter()
            .rev()
            .take(spark_area.width as usize)
            .rev()
            .filter_map(|s| match *kind {
                'E' => s.core_e_w.get(*idx).copied(),
                _ => s.core_p_w.get(*idx).copied(),
            })
            .map(|v| (v.max(0.0) * 1000.0) as u64)
            .collect();
        let spark_max = values.iter().copied().max().unwrap_or(1);
        graph::render_sparkline(frame, spark_area, &values, spark_max, color, theme);
    }
}
