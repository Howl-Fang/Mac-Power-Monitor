use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::App;
use crate::ui::theme::Theme;

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn render(frame: &mut Frame, area: Rect, app: &mut App, theme: &Theme) {
    let block = Block::bordered()
        .title(Line::from(vec![Span::styled("HELP", theme.title())]))
        .border_style(theme.border_style());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let system = app.sampler.system();
    let mut lines: Vec<Line<'static>> = Vec::new();
    let section = Style::default()
        .fg(theme.accent)
        .add_modifier(Modifier::BOLD);
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let dim = theme.dim_text();

    lines.push(line("MacPower ", bold, VERSION, bold));
    lines.push(line(
        "System: ",
        dim,
        &format!(
            "{} | {} ({}) | macOS {} ({})",
            system.hostname, system.chip, system.model, system.os_version, system.os_build
        ),
        theme.text(),
    ));
    lines.push(line(
        "Memory: ",
        dim,
        &format!(
            "{} | cores {} (P {} / E {}) | uptime {}s",
            crate::model::fmt_memory(system.memory_bytes),
            system.physical_cores,
            count(system.p_cores),
            count(system.e_cores),
            system.uptime_secs
        ),
        theme.text(),
    ));
    if system.rosetta {
        lines.push(Line::from(Span::styled(
            "Running under Rosetta. Build a native arm64 binary for full sensor access.",
            theme.warn(),
        )));
    }
    if let Some(snapshot) = &app.snapshot {
        let b = &snapshot.battery;
        if b.present {
            lines.push(section_line("BATTERY DETAILS", section));
            lines.push(line(
                "Capacity: ",
                bold,
                &format!(
                    "{} now | {} max | {} design | health {}",
                    capacity(b.current_capacity_mah),
                    capacity(b.max_capacity_mah),
                    capacity(b.design_capacity_mah),
                    crate::model::fmt_pct(b.health_pct)
                ),
                theme.text(),
            ));
            let adapter = b.adapter_name.as_deref().unwrap_or("n/a");
            let cells = if b.cell_voltages_v.is_empty() {
                "n/a".to_string()
            } else {
                b.cell_voltages_v
                    .iter()
                    .map(|v| format!("{v:.2} V"))
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            lines.push(line(
                "Hardware: ",
                bold,
                &format!("adapter {adapter} | cell voltages {cells}"),
                theme.text(),
            ));
            let sample = &snapshot.sample;
            lines.push(line(
                "Power: ",
                bold,
                &format!(
                    "{} fast | {} measured | {} fast current | {} measured current",
                    crate::model::fmt_battery_power(sample),
                    crate::model::fmt_power(b.power_w),
                    crate::model::fmt_battery_current(sample),
                    crate::model::fmt_current(b.amperage_a)
                ),
                theme.text(),
            ));
        }
    }
    lines.push(Line::from(""));

    lines.push(section_line("DATA SOURCES", section));
    lines.push(line(
        "SMC: ",
        bold,
        "PSTR (system power), PPBR (battery discharge power on this Mac), PDTR (DC input), PBLR/PDBR (display backlight when present).",
        theme.text(),
    ));
    lines.push(line(
        "Battery: ",
        bold,
        "AppleSmartBattery voltage, signed current, temperature, health, cycles. The current/power gauge refreshes only about every 30 seconds on this Mac.",
        theme.text(),
    ));
    lines.push(line(
        "Firmware: ",
        bold,
        "PowerTelemetryData SystemPowerIn/SystemLoad are last-resort whole-machine/input fallbacks when SMC keys are absent.",
        theme.text(),
    ));
    lines.push(line(
        "Components: ",
        bold,
        "IOReport Energy Model energy counters, diffed over the sample interval and converted to watts.",
        theme.text(),
    ));
    lines.push(line(
        "Display: ",
        bold,
        "IOReport DISP/DISPEXT is the SoC display controller; SMC PBLR/PDBR adds panel backlight power when the Mac exposes it.",
        theme.text(),
    ));
    lines.push(line(
        "Per-core: ",
        bold,
        "IOReport ECPU0..N / PCPU0..N channels when the chip exposes them; otherwise cluster totals only.",
        theme.text(),
    ));
    lines.push(Line::from(""));

    lines.push(section_line("METRICS AND FORMULAS", section));
    lines.push(line(
        "System power: ",
        bold,
        "on battery, battery discharge power first; on AC, SMC PSTR first, then PDTR, then firmware SystemLoad.",
        theme.text(),
    ));
    lines.push(line(
        "Battery power: ",
        bold,
        "signed: positive while charging, negative while discharging. ~ marks a fast SMC-derived estimate between gauge updates.",
        theme.text(),
    ));
    lines.push(line(
        "Fast battery: ",
        bold,
        "on battery use PPBR; while charging use (PDTR - PSTR) calibrated to the last fresh V x I gauge value. A clear adapter surplus is also used when macOS keeps IsCharging false. Raw/JSON keep the measured values too.",
        theme.text(),
    ));
    lines.push(line(
        "Power input: ",
        bold,
        "SMC PDTR first, then firmware SystemPowerIn, then battery V x I while charging.",
        theme.text(),
    ));
    lines.push(line(
        "CPU: ",
        bold,
        "CPU Energy, or ECPU+PCPU, or the sum of per-core channels. E/P are cluster totals.",
        theme.text(),
    ));
    lines.push(line(
        "Other: ",
        bold,
        "a partial sum of auxiliary rails (ISP/AVE/MSR/VDEC/SOC_AON/SOC_REST/PCIe); not a whole-machine total.",
        theme.text(),
    ));
    lines.push(line(
        "n/a: ",
        bold,
        "the sensor or channel is unavailable on this Mac. It does not mean zero.",
        theme.text(),
    ));
    lines.push(line(
        "Caveat: ",
        bold,
        "these values are estimates. Do not compare them across devices; use them to spot trends.",
        theme.text(),
    ));
    lines.push(Line::from(""));

    lines.push(section_line("HISTORY STORE", section));
    lines.push(line(
        "Rolling: ",
        bold,
        "the last 48 hours are written to /tmp/macpower/history as hourly TSV files. This uses a compact schema without the verbose rails column and older hours are pruned automatically.",
        theme.text(),
    ));
    lines.push(line(
        "Recording: ",
        bold,
        "press r to write a full raw TSV (with rails) under /tmp/macpower/recordings. Recording has no size limit. On stop the .part file is finalized as .tsv and is never appended to again; history before the recording stays in the rolling store.",
        theme.text(),
    ));
    lines.push(line(
        "History tab: ",
        bold,
        "1m/5m/15m/1h stay in the in-memory ring; 6h/24h/48h/All load from the rolling store and keep appending new samples. Series stack System, CPU, GPU, Display, DRAM, DC in, with the first series drawn on top. DC input is shown as 0 while on battery.",
        theme.text(),
    ));
    lines.push(line(
        "Export: ",
        bold,
        "e opens one page with a source selector and a time-scale x format grid. Arrow keys move the selected cell, Tab changes source, and Enter or the Export button writes the file into the current directory. Esc or a click on Cancel closes the page.",
        theme.text(),
    ));
    lines.push(Line::from(""));

    lines.push(section_line("KEYS", section));
    for (k, v) in [
        ("q / Ctrl-C", "quit"),
        ("? / Esc", "toggle help"),
        ("Tab / Shift-Tab / 1-5", "switch tabs"),
        ("h / l", "previous / next tab"),
        ("j / k / arrows", "scroll"),
        ("g / G", "top / bottom"),
        ("p", "pause / resume sampling"),
        ("r", "start / stop full raw recording"),
        ("e", "export history (source x time x format)"),
        ("+ / -", "change sample interval"),
        ("t", "cycle history window"),
        ("m", "toggle mouse capture"),
    ] {
        lines.push(line(&format!("{k:<24}"), theme.selected(), v, theme.text()));
    }
    lines.push(Line::from(""));

    lines.push(section_line("MOUSE", section));
    lines.push(Line::from(Span::styled(
        "Click tabs, footer buttons, history window buttons, export source arrows, time/format cells, Export and Cancel. Scroll over the content to scroll or change the history window.",
        theme.text(),
    )));
    lines.push(Line::from(Span::styled(
        "Mouse capture can interfere with terminal text selection; press m or use --no-mouse.",
        dim,
    )));
    lines.push(Line::from(""));

    lines.push(section_line("TSV AND PDF EXPORTS", section));
    lines.push(Line::from(Span::styled(
        "PDF draws multi-page charts and summary tables; y-axes that cross zero get a labeled 0 baseline, and implausible DC-input spikes from SMC PDTR are filtered out. TSV preserves the selected source's data columns: recordings keep the full rails column, while rolling 48h history uses the compact schema. Both apply the chosen time scale. --raw <path> and --report <path> remain available from the command line.",
        theme.text(),
    )));
    lines.push(Line::from(""));

    lines.push(section_line("COMPATIBILITY", section));
    lines.push(Line::from(Span::styled(
        "Apple Silicon M1-M4 and Intel, macOS 12+. Only the M3/macOS 14.2 path is hardware-verified. SMC-only and battery-only fallbacks keep the monitor useful when IOReport is absent.",
        theme.text(),
    )));
    lines.push(Line::from(Span::styled(
        "IOReport and AppleSMC are private APIs. Use direct distribution, not the Mac App Store. No root is required.",
        dim,
    )));
    lines.push(Line::from(Span::styled(
        "Debug: --list-sensors dumps channels, --disable smc|ioreport|battery tests fallbacks, --demo generates synthetic data.",
        dim,
    )));
    lines.push(Line::from(""));

    if !app.warnings.is_empty() {
        lines.push(section_line("WARNINGS", section));
        for warning in &app.warnings {
            lines.push(Line::from(Span::styled(warning.clone(), theme.warn())));
        }
        lines.push(Line::from(""));
    }

    lines.push(Line::from(Span::styled(
        "Press j/k or scroll to read. Esc returns to the previous tab.",
        dim,
    )));

    let paragraph = Paragraph::new(lines)
        .style(theme.text())
        .wrap(Wrap { trim: false })
        .scroll((app.help_scroll.min(u16::MAX as usize) as u16, 0));
    frame.render_widget(paragraph, inner);
}

fn capacity(mah: Option<u32>) -> String {
    match mah {
        Some(v) => format!("{v} mAh"),
        None => "n/a".to_string(),
    }
}

fn count(value: Option<usize>) -> String {
    match value {
        Some(v) => v.to_string(),
        None => "n/a".to_string(),
    }
}

fn line(prefix: &str, prefix_style: Style, rest: &str, style: Style) -> Line<'static> {
    Line::from(vec![
        Span::styled(prefix.to_string(), prefix_style),
        Span::styled(rest.to_string(), style),
    ])
}

fn section_line(text: &str, style: Style) -> Line<'static> {
    Line::from(Span::styled(text.to_string(), style))
}
