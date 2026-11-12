use std::time::Duration;

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::app::{window_label, Action, App, HISTORY_WINDOWS};
use crate::history::{series_avg, Summary};
use crate::model::{fmt_power_val, fmt_wh, plausible_dc_input_w, Sample};
use crate::ui::theme::Theme;
use crate::ui::widgets::graph::{self, GraphOptions, GraphSeries};

pub fn render(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    let [buttons, main, legend, battery_area, stats] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
        Constraint::Length(6),
        Constraint::Length(2),
    ])
    .areas(area);

    render_window_buttons(frame, buttons, app, theme);

    let samples: Vec<&Sample> = if let Some(cache) = app
        .history_cache
        .as_ref()
        .filter(|cache| cache.window_secs == app.window_secs)
    {
        cache.samples.iter().collect()
    } else if app.window_secs == 0 {
        app.history.all()
    } else {
        app.history.window(Duration::from_secs(app.window_secs))
    };
    let width = (main.width.saturating_sub(2) as usize * 2).max(60);
    let series = vec![
        GraphSeries {
            label: "System",
            color: theme.accent,
            data: series_avg(&samples, width, |s| s.system_w),
        },
        GraphSeries {
            label: "CPU",
            color: theme.green,
            data: series_avg(&samples, width, |s| s.cpu_w),
        },
        GraphSeries {
            label: "GPU",
            color: theme.orange,
            data: series_avg(&samples, width, |s| s.gpu_w),
        },
        GraphSeries {
            label: "Display",
            color: theme.yellow,
            data: series_avg(&samples, width, |s| s.display_w),
        },
        GraphSeries {
            label: "DRAM",
            color: theme.red,
            data: series_avg(&samples, width, |s| s.dram_w),
        },
        GraphSeries {
            label: "DC in",
            color: theme.blue,
            data: series_avg(&samples, width, |s| {
                Some(plausible_dc_input_w(s).unwrap_or(0.0))
            }),
        },
    ];
    graph::render_graph(
        frame,
        main,
        &format!("Power history ({})", window_label(app.window_secs)),
        &series,
        GraphOptions::power(),
        theme,
        app.ascii,
    );
    frame.render_widget(
        Paragraph::new(graph::legend_line(&series, theme, app.ascii)).style(theme.text()),
        legend,
    );

    let battery_series = vec![GraphSeries {
        label: "Battery %",
        color: theme.green,
        data: series_avg(&samples, width, |s| s.battery_pct),
    }];
    graph::render_graph(
        frame,
        battery_area,
        "Battery charge (%)",
        &battery_series,
        GraphOptions::percent(),
        theme,
        app.ascii,
    );

    let summary = Summary::from_samples(&samples);
    let line1 = format!(
        "System  min {}  avg {}  max {}   |   CPU avg {}   |   GPU avg {}",
        option_power(summary.system.min),
        option_power(summary.system.avg),
        option_power(summary.system.max),
        option_power(summary.cpu.avg),
        option_power(summary.gpu.avg)
    );
    let line2 = format!(
        "Energy  system {}  battery {}   |   samples {}   |   window {}",
        fmt_wh(summary.system.energy_wh),
        fmt_wh(summary.battery.energy_wh),
        summary.count,
        window_label(app.window_secs)
    );
    let [s1, s2] = Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(stats);
    frame.render_widget(Paragraph::new(line1).style(theme.text()), s1);
    frame.render_widget(Paragraph::new(line2).style(theme.dim_text()), s2);
}

fn render_window_buttons(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    let labels: Vec<String> = HISTORY_WINDOWS
        .iter()
        .map(|secs| format!(" {} ", window_label(*secs)))
        .collect();
    let widths: Vec<Constraint> = labels
        .iter()
        .map(|label| Constraint::Length(label.chars().count() as u16))
        .chain(std::iter::once(Constraint::Min(0)))
        .collect();
    let rects = Layout::horizontal(widths).split(area);
    for (i, secs) in HISTORY_WINDOWS.iter().enumerate() {
        let selected = app.window_secs == *secs;
        let style = if selected {
            theme.selected()
        } else {
            theme.dim_text()
        };
        frame.render_widget(
            Paragraph::new(labels[i].as_str())
                .style(style)
                .alignment(Alignment::Center),
            rects[i],
        );
        app.hit_zones.push((rects[i], Action::SetWindow(*secs)));
    }
    frame.render_widget(
        Paragraph::new("t cycle window")
            .style(theme.dim_text())
            .alignment(Alignment::Right),
        rects[HISTORY_WINDOWS.len()],
    );
}

fn option_power(value: Option<f32>) -> String {
    value
        .map(fmt_power_val)
        .unwrap_or_else(|| "n/a".to_string())
}
