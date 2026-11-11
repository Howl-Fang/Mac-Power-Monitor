//! Vector PDF report generation with printpdf 0.12.

use std::path::Path;

use chrono::{DateTime, Local};
use printpdf::{
    BuiltinFont, Color, Line, LinePoint, Mm, Op, PaintMode, PdfDocument, PdfFontHandle, PdfPage,
    PdfSaveOptions, Point, Polygon, PolygonRing, Pt, Rgb, TextItem, WindingOrder,
};

use crate::history::raw::is_top_level_rail;
use crate::history::{series_avg, MetricStats, Summary};
use crate::model::{fmt_power_val, fmt_wh, RailSource, RailValue, Sample, SystemInfo};

const PAGE_W_MM: f64 = 297.0;
const PAGE_H_MM: f64 = 210.0;
const MARGIN: f64 = 36.0;
const VERSION: &str = env!("CARGO_PKG_VERSION");

fn pt(v: f64) -> Pt {
    Pt(v as f32)
}

fn mm(v: f64) -> Mm {
    Mm(v as f32)
}

#[derive(Clone, Copy)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self { x, y, w, h }
    }
}

struct Series {
    label: String,
    color: Color,
    points: Vec<(f64, f64)>,
}

type SampleAccessor = fn(&Sample) -> Option<f32>;

#[derive(Clone, Copy)]
struct ChartOptions {
    y_min: Option<f64>,
    y_max: Option<f64>,
    show_legend: bool,
}

#[derive(Clone, Copy)]
struct ChartPalette<'a> {
    black: &'a Color,
    gray: &'a Color,
    light: &'a Color,
}

pub struct ReportInput<'a> {
    pub samples: &'a [Sample],
    pub system: &'a SystemInfo,
    pub rails: &'a [RailValue],
    pub source_label: String,
}

pub fn write_report(path: &Path, input: &ReportInput<'_>) -> Result<(), String> {
    if input.samples.is_empty() {
        return Err("no samples to export".to_string());
    }
    let refs: Vec<&Sample> = input.samples.iter().collect();
    let summary = Summary::from_samples(&refs);

    let page1 = build_page1(&refs, &summary, input);
    let page2 = build_page2(&refs, input);
    let page3 = build_page3(input, &summary);

    let pages = vec![
        PdfPage::new(mm(PAGE_W_MM), mm(PAGE_H_MM), page1),
        PdfPage::new(mm(PAGE_W_MM), mm(PAGE_H_MM), page2),
        PdfPage::new(mm(PAGE_W_MM), mm(PAGE_H_MM), page3),
    ];
    let bytes = PdfDocument::new("MacPower Report")
        .with_pages(pages)
        .save(&PdfSaveOptions::default(), &mut Vec::new());
    std::fs::write(path, bytes).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

fn build_page1(samples: &[&Sample], summary: &Summary, input: &ReportInput<'_>) -> Vec<Op> {
    let mut ops = Vec::new();
    let black = rgb(0.08, 0.09, 0.1);
    let gray = rgb(0.55, 0.58, 0.6);
    let light = rgb(0.88, 0.9, 0.92);
    let palette = ChartPalette {
        black: &black,
        gray: &gray,
        light: &light,
    };

    text(&mut ops, MARGIN, 560.0, 24.0, "MacPower Report", &black);
    let subtitle = format!(
        "{}  |  {}  |  macOS {} ({})",
        ascii(&input.system.hostname),
        ascii(&input.system.chip),
        ascii(&input.system.os_version),
        ascii(&input.system.os_build)
    );
    text(&mut ops, MARGIN, 540.0, 10.0, &subtitle, &gray);
    let range = format!(
        "{}  ->  {}   ({} samples, {})   source: {}",
        samples
            .first()
            .map(|s| fmt_time(s.ts))
            .unwrap_or_else(|| "n/a".to_string()),
        samples
            .last()
            .map(|s| fmt_time(s.ts))
            .unwrap_or_else(|| "n/a".to_string()),
        samples.len(),
        summary
            .duration_secs()
            .map(fmt_duration)
            .unwrap_or_else(|| "n/a".to_string()),
        ascii(&input.source_label)
    );
    text(&mut ops, MARGIN, 526.0, 8.0, &range, &gray);

    draw_summary_table(&mut ops, MARGIN, 500.0, 330.0, summary, &black, &light);

    let system_series = vec![
        Series {
            label: "System".to_string(),
            color: rgb(0.12, 0.55, 0.9),
            points: series_avg(samples, 420, |s| s.system_w),
        },
        Series {
            label: "DC input".to_string(),
            color: rgb(0.95, 0.55, 0.12),
            points: series_avg(samples, 420, |s| s.dc_in_w),
        },
        Series {
            label: "Battery".to_string(),
            color: rgb(0.2, 0.7, 0.35),
            points: series_avg(samples, 420, |s| s.battery_w),
        },
    ];
    draw_chart(
        &mut ops,
        Rect::new(400.0, 350.0, 405.0, 155.0),
        "System / input / battery (W)",
        &system_series,
        ChartOptions {
            y_min: None,
            y_max: None,
            show_legend: true,
        },
        samples,
        &palette,
    );

    let battery_series = vec![Series {
        label: "Battery".to_string(),
        color: rgb(0.2, 0.7, 0.35),
        points: series_avg(samples, 420, |s| s.battery_pct),
    }];
    draw_chart(
        &mut ops,
        Rect::new(400.0, 200.0, 405.0, 95.0),
        "Battery charge (%)",
        &battery_series,
        ChartOptions {
            y_min: Some(0.0),
            y_max: Some(100.0),
            show_legend: false,
        },
        samples,
        &palette,
    );

    footer(&mut ops, 1, &gray);
    ops
}

fn build_page2(samples: &[&Sample], input: &ReportInput<'_>) -> Vec<Op> {
    let mut ops = Vec::new();
    let black = rgb(0.08, 0.09, 0.1);
    let gray = rgb(0.55, 0.58, 0.6);
    let light = rgb(0.88, 0.9, 0.92);
    let palette = ChartPalette {
        black: &black,
        gray: &gray,
        light: &light,
    };

    text(
        &mut ops,
        MARGIN,
        560.0,
        20.0,
        "CPU and component power",
        &black,
    );

    let cpu_series = vec![
        Series {
            label: "CPU".to_string(),
            color: rgb(0.2, 0.7, 0.35),
            points: series_avg(samples, 380, |s| s.cpu_w),
        },
        Series {
            label: "E-cluster".to_string(),
            color: rgb(0.2, 0.55, 0.9),
            points: series_avg(samples, 380, |s| s.ecpu_w),
        },
        Series {
            label: "P-cluster".to_string(),
            color: rgb(0.65, 0.35, 0.8),
            points: series_avg(samples, 380, |s| s.pcpu_w),
        },
    ];
    draw_chart(
        &mut ops,
        Rect::new(MARGIN, 330.0, 370.0, 195.0),
        "CPU clusters (W)",
        &cpu_series,
        ChartOptions {
            y_min: None,
            y_max: None,
            show_legend: true,
        },
        samples,
        &palette,
    );

    let mini: [(&str, Color, SampleAccessor); 5] = [
        ("GPU", rgb(0.95, 0.6, 0.15), |s: &Sample| s.gpu_w),
        ("ANE", rgb(0.7, 0.4, 0.8), |s: &Sample| s.ane_w),
        ("DRAM", rgb(0.85, 0.25, 0.25), |s: &Sample| s.dram_w),
        ("Display", rgb(0.9, 0.75, 0.15), |s: &Sample| s.display_w),
        ("Other rails", rgb(0.5, 0.55, 0.6), |s: &Sample| s.other_w),
    ];
    let mut y = 469.0;
    for (label, color, accessor) in mini {
        let series = vec![Series {
            label: label.to_string(),
            color,
            points: series_avg(samples, 300, accessor),
        }];
        draw_chart(
            &mut ops,
            Rect::new(420.0, y, 385.0, 70.0),
            &format!("{label} (W)"),
            &series,
            ChartOptions {
                y_min: None,
                y_max: None,
                show_legend: false,
            },
            samples,
            &palette,
        );
        y -= 85.0;
    }

    if let Some(last) = samples.last() {
        draw_core_bars(
            &mut ops,
            Rect::new(MARGIN, 100.0, 370.0, 195.0),
            last,
            &black,
            &gray,
        );
    }

    let note = format!(
        "Chip: {}  |  physical cores: {}  |  P/E: {}/{}",
        ascii(&input.system.chip),
        input.system.physical_cores,
        input
            .system
            .p_cores
            .map(|v| v.to_string())
            .unwrap_or_else(|| "n/a".into()),
        input
            .system
            .e_cores
            .map(|v| v.to_string())
            .unwrap_or_else(|| "n/a".into())
    );
    text(&mut ops, MARGIN, 82.0, 8.0, &note, &gray);
    footer(&mut ops, 2, &gray);
    ops
}

fn build_page3(input: &ReportInput<'_>, summary: &Summary) -> Vec<Op> {
    let mut ops = Vec::new();
    let black = rgb(0.08, 0.09, 0.1);
    let gray = rgb(0.55, 0.58, 0.6);
    let light = rgb(0.88, 0.9, 0.92);

    text(
        &mut ops,
        MARGIN,
        560.0,
        20.0,
        "Latest snapshot rails and notes",
        &black,
    );
    text(
        &mut ops,
        MARGIN,
        542.0,
        8.0,
        "Current snapshot values in watts; per-core rails are charted on page 2. RAW UNIT is the original IOReport unit.",
        &gray,
    );

    let mut rows: Vec<&RailValue> = input
        .rails
        .iter()
        .filter(|r| is_top_level_rail(&r.name) && !is_per_core_rail(&r.name))
        .collect();
    rows.sort_by_key(|r| source_rank(r.source));
    let total_rows = rows.len();
    rows.truncate(40);

    let x = MARGIN;
    let right_x = MARGIN + 395.0;
    let column_w = 360.0;
    let first_row_y = 497.0;
    let row_h = 15.0;
    let mid = rows.len().div_ceil(2);
    if !rows.is_empty() {
        for (col_x, chunk) in [(x, &rows[..mid]), (right_x, &rows[mid..])] {
            let mut y = 515.0;
            text(&mut ops, col_x, y, 8.0, "RAIL", &black);
            text(&mut ops, col_x + 230.0, y, 8.0, "POWER", &black);
            text(&mut ops, col_x + 300.0, y, 8.0, "RAW UNIT", &black);
            text(&mut ops, col_x + 345.0, y, 8.0, "SOURCE", &black);
            y -= 6.0;
            line(&mut ops, col_x, y, col_x + column_w, y, &light, 0.5);
            y -= 12.0;
            for rail in chunk {
                text(&mut ops, col_x, y, 7.5, &ascii(&rail.name), &black);
                text(
                    &mut ops,
                    col_x + 230.0,
                    y,
                    7.5,
                    &fmt_power_val(rail.w),
                    &black,
                );
                text(&mut ops, col_x + 300.0, y, 7.5, &ascii(&rail.unit), &gray);
                text(&mut ops, col_x + 345.0, y, 7.5, rail.source.label(), &gray);
                y -= row_h;
            }
        }
    }

    let mut y = first_row_y - mid as f64 * row_h - 6.0;
    if rows.is_empty() {
        text(
            &mut ops,
            x,
            y,
            8.0,
            "No IOReport/SMC rails available.",
            &gray,
        );
        y -= 14.0;
    }
    let counts = format!(
        "Totals: system avg {}, CPU avg {}, GPU avg {}, DRAM avg {}. Energy (system): {}.",
        metric_avg(&summary.system),
        metric_avg(&summary.cpu),
        metric_avg(&summary.gpu),
        metric_avg(&summary.dram),
        fmt_wh(summary.system.energy_wh)
    );
    for line_text in wrap_ascii(&counts, 110) {
        text(&mut ops, x, y, 8.0, &line_text, &black);
        y -= 12.0;
    }
    if total_rows > rows.len() {
        text(
            &mut ops,
            x,
            y,
            7.0,
            &format!(
                "{} additional rails omitted from this table.",
                total_rows - rows.len()
            ),
            &gray,
        );
        y -= 12.0;
    }

    y -= 6.0;
    let notes = [
        "Data sources: SMC PSTR/PPBR/PDTR (whole-machine/battery rails), AppleSmartBattery (voltage, current, battery power), IOReport Energy Model (component energy counters).",
        "Component values are derived from energy deltas over the sample interval and are estimates. They are not calibrated for cross-device comparison.",
        "Battery power is signed: positive while charging, negative while discharging. SMC PSTR is system power and can differ from battery-side power.",
        "Private APIs (IOReport, AppleSMC) are used; this report is intended for local diagnostics, not App Store distribution.",
    ];
    for note in notes {
        for line_text in wrap_ascii(note, 118) {
            text(&mut ops, x, y, 7.0, &line_text, &gray);
            y -= 10.0;
        }
        y -= 2.0;
    }

    footer(&mut ops, 3, &gray);
    ops
}

fn is_per_core_rail(name: &str) -> bool {
    let Some(rest) = name
        .strip_prefix("ECPU")
        .or_else(|| name.strip_prefix("PCPU"))
    else {
        return false;
    };
    !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit())
}

fn source_rank(source: RailSource) -> u8 {
    match source {
        RailSource::Smc => 0,
        RailSource::IoKit => 1,
        RailSource::Firmware => 2,
        RailSource::IoReport => 3,
    }
}

fn draw_summary_table(
    ops: &mut Vec<Op>,
    x: f64,
    y: f64,
    w: f64,
    summary: &Summary,
    black: &Color,
    light: &Color,
) {
    let rows: [(&str, &MetricStats); 10] = [
        ("System", &summary.system),
        ("Battery (signed)", &summary.battery),
        ("CPU", &summary.cpu),
        ("E-cluster", &summary.ecpu),
        ("P-cluster", &summary.pcpu),
        ("GPU", &summary.gpu),
        ("ANE", &summary.ane),
        ("DRAM", &summary.dram),
        ("Display", &summary.display),
        ("Other rails", &summary.other),
    ];

    let cols = [x, x + 110.0, x + 170.0, x + 230.0, x + 290.0];
    text(ops, cols[0], y, 8.0, "METRIC", black);
    text(ops, cols[1], y, 8.0, "MIN", black);
    text(ops, cols[2], y, 8.0, "AVG", black);
    text(ops, cols[3], y, 8.0, "MAX", black);
    text(ops, cols[4], y, 8.0, "ENERGY", black);
    let mut line_y = y - 5.0;
    line(ops, x, line_y, x + w, line_y, light, 0.5);
    line_y -= 13.0;

    for (name, stats) in rows {
        text(ops, cols[0], line_y, 7.0, name, black);
        text(ops, cols[1], line_y, 7.0, &stat_min(stats), black);
        text(ops, cols[2], line_y, 7.0, &stat_avg(stats), black);
        text(ops, cols[3], line_y, 7.0, &stat_max(stats), black);
        text(ops, cols[4], line_y, 7.0, &fmt_wh(stats.energy_wh), black);
        line_y -= 13.0;
    }
    line(ops, x, line_y + 4.0, x + w, line_y + 4.0, light, 0.5);
}

fn draw_chart(
    ops: &mut Vec<Op>,
    rect: Rect,
    title: &str,
    series: &[Series],
    options: ChartOptions,
    samples: &[&Sample],
    palette: &ChartPalette<'_>,
) {
    text(
        ops,
        rect.x,
        rect.y + rect.h + 13.0,
        8.0,
        &format!(
            "{title}  [{} -> {}]",
            fmt_time_short(samples.first().map(|s| s.ts)),
            fmt_time_short(samples.last().map(|s| s.ts))
        ),
        palette.black,
    );

    let px = rect.x + 38.0;
    let py = rect.y + 18.0;
    let pw = (rect.w - 44.0).max(10.0);
    let ph = (rect.h - 26.0).max(10.0);

    let mut data_min = f64::INFINITY;
    let mut data_max = f64::NEG_INFINITY;
    for s in series {
        for (_, y) in &s.points {
            data_min = data_min.min(*y);
            data_max = data_max.max(*y);
        }
    }
    let mut y_min = options.y_min.unwrap_or(if data_min.is_finite() {
        data_min.min(0.0)
    } else {
        0.0
    });
    let mut y_max = options
        .y_max
        .unwrap_or(if data_max.is_finite() { data_max } else { 1.0 });
    if !y_min.is_finite() || !y_max.is_finite() {
        y_min = 0.0;
        y_max = 1.0;
    }
    if y_max <= y_min {
        y_max = y_min + 1.0;
    }
    let headroom = (y_max - y_min) * 0.08;
    if options.y_max.is_none() {
        y_max += headroom;
    }
    if y_min < 0.0 && options.y_min.is_none() {
        y_min -= headroom;
    }

    // Horizontal grid and y labels.
    let grid = 4;
    for i in 0..=grid {
        let t = i as f64 / grid as f64;
        let gy = py + ph * t;
        line(ops, px, gy, px + pw, gy, palette.light, 0.4);
        let value = y_min + (y_max - y_min) * t;
        let label = if (y_max - y_min).abs() < 2.0 {
            format!("{value:.2}")
        } else {
            format!("{value:.0}")
        };
        text(ops, rect.x, gy - 2.5, 6.5, &label, palette.gray);
    }
    line(ops, px, py, px, py + ph, palette.gray, 0.7);
    line(ops, px, py, px + pw, py, palette.gray, 0.7);

    text(
        ops,
        px,
        rect.y + 4.0,
        6.5,
        &fmt_time_short(samples.first().map(|s| s.ts)),
        palette.gray,
    );
    let end = fmt_time_short(samples.last().map(|s| s.ts));
    let end_x = px + pw - end.len() as f64 * 3.6;
    text(ops, end_x.max(px), rect.y + 4.0, 6.5, &end, palette.gray);

    if series.iter().all(|s| s.points.is_empty()) {
        text(
            ops,
            px + pw * 0.35,
            py + ph * 0.5,
            8.0,
            "no data",
            palette.gray,
        );
        return;
    }

    for s in series {
        if s.points.len() < 2 {
            continue;
        }
        let points: Vec<LinePoint> = s
            .points
            .iter()
            .map(|(x, y)| {
                let px_x = px + x.clamp(0.0, 1.0) * pw;
                let py_y = py + ((y.clamp(y_min, y_max) - y_min) / (y_max - y_min)) * ph;
                LinePoint {
                    p: Point {
                        x: pt(px_x),
                        y: pt(py_y),
                    },
                    bezier: false,
                }
            })
            .collect();
        line_shape(ops, points, &s.color, 1.1);
    }

    if options.show_legend {
        let mut ly = py + ph - 8.0;
        for s in series {
            if s.points.is_empty() {
                continue;
            }
            fill_rect(ops, px + pw - 95.0, ly - 2.0, 5.0, 5.0, &s.color);
            text(
                ops,
                px + pw - 87.0,
                ly - 2.0,
                6.5,
                &ascii(&s.label),
                palette.black,
            );
            ly -= 9.0;
        }
    }
}

fn draw_core_bars(ops: &mut Vec<Op>, rect: Rect, sample: &Sample, black: &Color, gray: &Color) {
    text(
        ops,
        rect.x,
        rect.y + rect.h + 13.0,
        9.0,
        "Per-core power (latest, mW)",
        black,
    );

    let mut cores: Vec<(String, f32, Color)> = Vec::new();
    for (i, w) in sample.core_e_w.iter().enumerate() {
        cores.push((format!("E{i}"), *w, rgb(0.2, 0.55, 0.9)));
    }
    for (i, w) in sample.core_p_w.iter().enumerate() {
        cores.push((format!("P{i}"), *w, rgb(0.65, 0.35, 0.8)));
    }
    if cores.is_empty() {
        text(
            ops,
            rect.x,
            rect.y + rect.h * 0.6,
            8.0,
            "Per-core channels unavailable on this Mac.",
            gray,
        );
        return;
    }
    let max = cores
        .iter()
        .map(|(_, w, _)| *w)
        .fold(0.0f32, f32::max)
        .max(0.001);
    let bar_left = rect.x + 34.0;
    let bar_width = (rect.w - 120.0).max(10.0);
    let mut y = rect.y + rect.h - 12.0;
    for (label, w, color) in cores.iter().take(16) {
        text(ops, rect.x, y, 7.0, label, black);
        let width = (*w / max).clamp(0.0, 1.0) as f64 * bar_width;
        fill_rect(ops, bar_left, y - 2.0, width.max(0.5), 8.0, color);
        text(
            ops,
            bar_left + width + 5.0,
            y,
            7.0,
            &format!("{:.1}", w * 1000.0),
            black,
        );
        y -= 15.0;
    }
}

fn footer(ops: &mut Vec<Op>, page: u32, gray: &Color) {
    let text_line = format!(
        "macpower {VERSION}  |  power values are estimates; sources: SMC, AppleSmartBattery, IOReport  |  page {page}/3"
    );
    text(ops, MARGIN, 20.0, 7.0, &text_line, gray);
}

fn text(ops: &mut Vec<Op>, x: f64, y: f64, size: f64, value: &str, color: &Color) {
    ops.push(Op::StartTextSection);
    // Drawing coordinates are in points; `Point::new` expects millimeters and
    // converts them to points, which would push text far off the page.
    ops.push(Op::SetTextCursor {
        pos: Point { x: pt(x), y: pt(y) },
    });
    ops.push(Op::SetFont {
        font: PdfFontHandle::Builtin(BuiltinFont::Helvetica),
        size: pt(size),
    });
    ops.push(Op::SetLineHeight { lh: pt(size) });
    ops.push(Op::SetFillColor { col: color.clone() });
    ops.push(Op::ShowText {
        items: vec![TextItem::Text(ascii(value))],
    });
    ops.push(Op::EndTextSection);
}

fn line(ops: &mut Vec<Op>, x1: f64, y1: f64, x2: f64, y2: f64, color: &Color, width: f64) {
    ops.push(Op::SetOutlineColor { col: color.clone() });
    ops.push(Op::SetOutlineThickness { pt: pt(width) });
    ops.push(Op::DrawLine {
        line: Line {
            points: vec![
                LinePoint {
                    p: Point {
                        x: pt(x1),
                        y: pt(y1),
                    },
                    bezier: false,
                },
                LinePoint {
                    p: Point {
                        x: pt(x2),
                        y: pt(y2),
                    },
                    bezier: false,
                },
            ],
            is_closed: false,
        },
    });
}

fn line_shape(ops: &mut Vec<Op>, points: Vec<LinePoint>, color: &Color, width: f64) {
    ops.push(Op::SetOutlineColor { col: color.clone() });
    ops.push(Op::SetOutlineThickness { pt: pt(width) });
    ops.push(Op::DrawLine {
        line: Line {
            points,
            is_closed: false,
        },
    });
}

fn fill_rect(ops: &mut Vec<Op>, x: f64, y: f64, w: f64, h: f64, color: &Color) {
    ops.push(Op::SetFillColor { col: color.clone() });
    let points = vec![
        LinePoint {
            p: Point { x: pt(x), y: pt(y) },
            bezier: false,
        },
        LinePoint {
            p: Point {
                x: pt(x + w),
                y: pt(y),
            },
            bezier: false,
        },
        LinePoint {
            p: Point {
                x: pt(x + w),
                y: pt(y + h),
            },
            bezier: false,
        },
        LinePoint {
            p: Point {
                x: pt(x),
                y: pt(y + h),
            },
            bezier: false,
        },
    ];
    ops.push(Op::DrawPolygon {
        polygon: Polygon {
            rings: vec![PolygonRing { points }],
            mode: PaintMode::Fill,
            winding_order: WindingOrder::NonZero,
        },
    });
}

fn rgb(r: f32, g: f32, b: f32) -> Color {
    Color::Rgb(Rgb {
        r,
        g,
        b,
        icc_profile: None,
    })
}

fn ascii(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                '?'
            }
        })
        .collect()
}

fn wrap_ascii(s: &str, width: usize) -> Vec<String> {
    let s = ascii(s);
    let mut out = Vec::new();
    let mut current = String::new();
    for word in s.split_whitespace() {
        if !current.is_empty() && current.len() + 1 + word.len() > width {
            out.push(current);
            current = String::new();
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn fmt_time(ts: std::time::SystemTime) -> String {
    DateTime::<Local>::from(ts)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

fn fmt_time_short(ts: Option<std::time::SystemTime>) -> String {
    ts.map(|ts| DateTime::<Local>::from(ts).format("%H:%M:%S").to_string())
        .unwrap_or_else(|| "--:--:--".to_string())
}

fn fmt_duration(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h {m:02}m {s:02}s")
    } else {
        format!("{m}m {s:02}s")
    }
}

fn stat_min(stats: &MetricStats) -> String {
    stats
        .min
        .map(fmt_power_val)
        .unwrap_or_else(|| "n/a".to_string())
}

fn stat_avg(stats: &MetricStats) -> String {
    stats
        .avg
        .map(fmt_power_val)
        .unwrap_or_else(|| "n/a".to_string())
}

fn stat_max(stats: &MetricStats) -> String {
    stats
        .max
        .map(fmt_power_val)
        .unwrap_or_else(|| "n/a".to_string())
}

fn metric_avg(stats: &MetricStats) -> String {
    stats
        .avg
        .map(fmt_power_val)
        .unwrap_or_else(|| "n/a".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PowerSource;
    use std::time::{Duration, UNIX_EPOCH};

    fn samples(n: usize) -> Vec<Sample> {
        (0..n)
            .map(|i| Sample {
                ts: UNIX_EPOCH + Duration::from_secs(i as u64),
                system_w: Some(4.0 + (i % 5) as f32 * 0.2),
                smc_pstr_w: Some(3.9),
                dc_in_w: Some(0.0),
                battery_w: Some(-4.1),
                battery_pct: Some(70.0 - i as f32 * 0.1),
                cpu_w: Some(0.1 + (i % 3) as f32 * 0.01),
                ecpu_w: Some(0.06),
                pcpu_w: Some(0.04),
                core_e_w: vec![0.015, 0.016],
                core_p_w: vec![0.011],
                gpu_w: Some(0.0),
                ane_w: Some(0.0),
                dram_w: Some(0.06),
                display_w: Some(0.02),
                other_w: Some(0.08),
                source: PowerSource::Battery,
                ..Sample::default()
            })
            .collect()
    }

    #[test]
    fn writes_a_three_page_pdf() {
        let samples = samples(20);
        let system = SystemInfo {
            hostname: "test".into(),
            chip: "Apple M3".into(),
            model: "Mac15,3".into(),
            os_version: "14.2".into(),
            os_build: "23C64".into(),
            physical_cores: 8,
            p_cores: Some(4),
            e_cores: Some(4),
            memory_bytes: 8 << 30,
            uptime_secs: 100,
            rosetta: false,
        };
        let rails = vec![RailValue {
            name: "CPU Energy".into(),
            group: "Energy Model".into(),
            w: 0.13,
            unit: "mJ".into(),
            source: crate::model::RailSource::IoReport,
        }];
        let input = ReportInput {
            samples: &samples,
            system: &system,
            rails: &rails,
            source_label: "test".into(),
        };
        let path = std::env::temp_dir().join(format!("macpower-test-{}.pdf", std::process::id()));
        write_report(&path, &input).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert!(bytes.starts_with(b"%PDF"));
        assert!(bytes.len() > 1000);
        let doc = lopdf::Document::load_mem(&bytes).unwrap();
        assert_eq!(doc.get_pages().len(), 3);
    }

    #[test]
    fn text_cursor_uses_point_coordinates() {
        let mut ops = Vec::new();
        text(
            &mut ops,
            MARGIN,
            560.0,
            24.0,
            "MacPower Report",
            &rgb(0.0, 0.0, 0.0),
        );
        let Op::SetTextCursor { pos } = &ops[1] else {
            panic!("expected text cursor op");
        };
        assert!((pos.x.0 - MARGIN as f32).abs() < 0.01);
        assert!((pos.y.0 - 560.0).abs() < 0.01);
    }

    #[test]
    fn sanitizes_non_ascii() {
        assert_eq!(ascii("a°b\tc"), "a?b?c");
        assert_eq!(wrap_ascii("one two three", 7), vec!["one two", "three"]);
    }
}
