use std::time::Duration;

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use ratatui::Frame;

use crate::app::App;
use crate::history::series_avg;
use crate::model::{fmt_power, Sample};
use crate::ui::theme::Theme;
use crate::ui::widgets::bar::{render_bar_row, render_meter_bar};
use crate::ui::widgets::graph::{self, GraphOptions, GraphSeries};

pub fn render(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    let Some(snapshot) = app.snapshot.as_ref() else {
        frame.render_widget(
            Paragraph::new("Waiting for the first sample...")
                .style(theme.dim_text())
                .alignment(ratatui::layout::Alignment::Center),
            area,
        );
        return;
    };
    let warming = app.history.len() <= 1 && snapshot.sample.cpu_w.is_none();
    let core_count =
        (snapshot.sample.core_e_w.len() + snapshot.sample.core_p_w.len()).max(1) as u16;

    // Show the history chart only when every core still fits alongside it.
    if area.height >= core_count + 12 && area.width >= 80 {
        let [clusters, history, cores] = Layout::vertical([
            Constraint::Length(5),
            Constraint::Min(5),
            Constraint::Length(core_count + 2),
        ])
        .areas(area);
        render_clusters(frame, clusters, snapshot, warming, theme);
        render_history(frame, history, app, theme);
        render_core_panel(frame, cores, &snapshot.sample, app, theme, warming);
    } else {
        let [clusters, cores] =
            Layout::vertical([Constraint::Length(5), Constraint::Min(0)]).areas(area);
        render_clusters(frame, clusters, snapshot, warming, theme);
        render_core_panel(frame, cores, &snapshot.sample, app, theme, warming);
    }
}

fn render_clusters(
    frame: &mut Frame,
    area: Rect,
    snapshot: &crate::model::Snapshot,
    warming: bool,
    theme: &Theme,
) {
    let block = Block::bordered()
        .title(Line::from(vec![Span::styled(
            "CPU CLUSTERS",
            theme.title(),
        )]))
        .border_style(theme.border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height < 3 {
        return;
    }
    if warming {
        frame.render_widget(
            Paragraph::new("Warming up IOReport counters (needs two samples)...")
                .style(theme.warn()),
            inner,
        );
        return;
    }
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

fn render_history(frame: &mut Frame, area: Rect, app: &App, theme: &Theme) {
    let samples = app.history.window(Duration::from_secs(60));
    let width = ((area.width.saturating_sub(2) as usize) * 2).max(60);
    let series = vec![
        GraphSeries {
            label: "CPU",
            color: theme.green,
            data: series_avg(&samples, width, |s| s.cpu_w),
        },
        GraphSeries {
            label: "E",
            color: theme.blue,
            data: series_avg(&samples, width, |s| s.ecpu_w),
        },
        GraphSeries {
            label: "P",
            color: theme.purple,
            data: series_avg(&samples, width, |s| s.pcpu_w),
        },
    ];
    let options = GraphOptions {
        legend_in_title: true,
        ..GraphOptions::power()
    };
    graph::render_graph(
        frame,
        area,
        "CPU POWER HISTORY (60s)",
        &series,
        options,
        theme,
        app.ascii,
    );
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
    let bar_width = (inner.width / 4).clamp(14, 36);

    for (row_idx, (kind, idx, w)) in cores.iter().take(count).enumerate() {
        let row = rows[row_idx];
        let [label_area, bar_area, value_area, spark_area] = Layout::horizontal([
            Constraint::Length(6),
            Constraint::Length(bar_width),
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
        let ratio = (*w / max).clamp(0.0, 1.0) as f64;
        render_meter_bar(
            frame,
            bar_area,
            ratio,
            color,
            &format!("{:.0}%", ratio * 100.0),
            theme,
        );
        frame.render_widget(
            Paragraph::new(format!(" {}", fmt_power(Some(*w)))).style(theme.text()),
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
        graph::render_sparkline(frame, spark_area, &values, spark_max, color, app.ascii);
    }
}
