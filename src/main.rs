mod app;
mod collectors;
mod history;
mod model;
mod ui;

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::{Context, Result};
use chrono::{DateTime, Local, SecondsFormat};
use clap::{Parser, ValueEnum};
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind};
use crossterm::execute;
use serde_json::json;

use app::{App, AppOptions};
use collectors::{CollectorOptions, Sampler};
use history::pdf::{write_report, ReportInput};
use history::raw::{self, Recorder};
use model::{fmt_memory, Sample, Snapshot};
use ui::theme::Theme;

#[derive(Parser, Debug)]
#[command(name = "macpower", version, about = "Terminal power monitor for macOS")]
struct Cli {
    /// Sample interval in milliseconds.
    #[arg(short = 'i', long, default_value_t = 1000, value_name = "MS")]
    interval: u64,
    /// In-memory history length in seconds.
    #[arg(short = 'H', long, default_value_t = 7200, value_name = "SEC")]
    history: u64,
    /// Start raw TSV recording at PATH.
    #[arg(long, value_name = "PATH")]
    raw: Option<PathBuf>,
    /// Write a PDF report to PATH on exit.
    #[arg(long, value_name = "PATH")]
    report: Option<PathBuf>,
    /// Run headless for SEC seconds, then exit.
    #[arg(long, value_name = "SEC")]
    duration: Option<u64>,
    /// Print one sample and exit.
    #[arg(long)]
    once: bool,
    /// Output format for --once / --headless.
    #[arg(long, value_enum, default_value_t = Format::Tsv)]
    format: Format,
    /// Do not start the TUI.
    #[arg(long)]
    headless: bool,
    /// Dump detected SMC keys and IOReport channels.
    #[arg(long)]
    list_sensors: bool,
    /// Do not capture mouse events.
    #[arg(long)]
    no_mouse: bool,
    /// Monochrome UI.
    #[arg(long)]
    no_color: bool,
    /// ASCII graphs instead of braille/Unicode.
    #[arg(long)]
    ascii: bool,
    /// Disable a collector (repeatable).
    #[arg(long, value_enum, value_name = "WHAT")]
    disable: Vec<Disable>,
    /// Use synthetic data; do not touch hardware.
    #[arg(long)]
    demo: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum Format {
    Tsv,
    Json,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum Disable {
    Smc,
    Ioreport,
    Battery,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("macpower: {err:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let options = CollectorOptions {
        disable_smc: cli.disable.contains(&Disable::Smc),
        disable_ioreport: cli.disable.contains(&Disable::Ioreport),
        disable_battery: cli.disable.contains(&Disable::Battery),
    };
    let interval_ms = cli.interval.clamp(100, 10_000);
    let history_secs = cli.history.max(1);

    if cli.list_sensors {
        return run_list_sensors(&cli, options);
    }

    let stdout_is_tty = std::io::stdout().is_terminal();
    let auto_once = !stdout_is_tty && !cli.headless && cli.duration.is_none();
    if cli.once || auto_once {
        return run_once(&cli, options, interval_ms);
    }
    let headless = cli.headless || cli.duration.is_some() || !stdout_is_tty;
    if headless {
        return run_headless(&cli, options, interval_ms);
    }
    run_tui(&cli, options, interval_ms, history_secs)
}

fn make_sampler(cli: &Cli, options: CollectorOptions) -> Sampler {
    if cli.demo {
        Sampler::demo()
    } else {
        Sampler::real(options)
    }
}

fn run_list_sensors(cli: &Cli, options: CollectorOptions) -> Result<()> {
    let mut sampler = make_sampler(cli, options);
    let system = sampler.system();
    println!("MacPower {} sensor report", env!("CARGO_PKG_VERSION"));
    println!(
        "host={} chip={} model={} macos={} ({}) cores={} memory={}",
        system.hostname,
        system.chip,
        system.model,
        system.os_version,
        system.os_build,
        system.physical_cores,
        fmt_memory(system.memory_bytes)
    );
    for warning in sampler.warnings() {
        eprintln!("warning: {warning}");
    }
    print!("{}", sampler.sensor_report());
    Ok(())
}

fn run_once(cli: &Cli, options: CollectorOptions, interval_ms: u64) -> Result<()> {
    let mut sampler = make_sampler(cli, options);
    for warning in sampler.warnings() {
        eprintln!("warning: {warning}");
    }
    if !cli.demo {
        let _ = sampler.tick();
        std::thread::sleep(Duration::from_millis(interval_ms));
    }
    let snapshot = sampler.tick();
    match cli.format {
        Format::Tsv => print_tsv_header(),
        Format::Json => {}
    }
    print_snapshot(cli.format, &snapshot)?;

    if let Some(path) = &cli.raw {
        write_single_raw(path, &snapshot, sampler.system())?;
        eprintln!("raw: {}", path.display());
    }
    if let Some(path) = &cli.report {
        write_single_report(path, &snapshot, sampler.system())?;
        eprintln!("report: {}", path.display());
    }
    Ok(())
}

fn run_headless(cli: &Cli, options: CollectorOptions, interval_ms: u64) -> Result<()> {
    let mut sampler = make_sampler(cli, options);
    for warning in sampler.warnings() {
        eprintln!("warning: {warning}");
    }
    if !cli.demo {
        let _ = sampler.tick();
        std::thread::sleep(Duration::from_millis(interval_ms));
    }

    let mut recorder = Recorder::new();
    if let Some(path) = &cli.raw {
        recorder
            .start(path.clone(), sampler.system())
            .with_context(|| format!("cannot create {}", path.display()))?;
    }
    if cli.format == Format::Tsv {
        print_tsv_header();
    }

    let stop = install_signal_flag()?;
    let interval = Duration::from_millis(interval_ms);
    let started = Instant::now();
    let duration = cli.duration.map(|s| Duration::from_secs(s.max(1)));
    let mut samples: Vec<Sample> = Vec::new();
    let mut last_rails = Vec::new();

    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let snapshot = sampler.tick();
        samples.push(snapshot.sample.clone());
        last_rails = snapshot.rails.clone();
        if recorder.is_recording() {
            recorder
                .write(&snapshot)
                .with_context(|| "writing raw TSV")?;
        }
        print_snapshot(cli.format, &snapshot)?;
        std::io::stdout().flush().ok();

        if let Some(duration) = duration {
            if started.elapsed() >= duration {
                break;
            }
        }
        std::thread::sleep(interval);
    }

    let raw_path = recorder.stop();
    if let Some(path) = &raw_path {
        eprintln!("raw: {} ({} rows)", path.display(), recorder.rows());
    }
    if let Some(path) = &cli.report {
        if samples.is_empty() {
            anyhow::bail!("no samples collected for --report");
        }
        let input = ReportInput {
            samples: &samples,
            system: sampler.system(),
            rails: &last_rails,
            source_label: "headless session".to_string(),
        };
        write_report(path, &input).map_err(anyhow::Error::msg)?;
        eprintln!("report: {}", path.display());
    }
    eprintln!(
        "samples: {} over {:.1}s",
        samples.len(),
        started.elapsed().as_secs_f64()
    );
    Ok(())
}

fn run_tui(
    cli: &Cli,
    options: CollectorOptions,
    interval_ms: u64,
    history_secs: u64,
) -> Result<()> {
    let theme = Theme::new(cli.no_color);
    let mouse = !cli.no_mouse;
    let mut terminal = ratatui::init();
    install_panic_hook();
    let guard = TerminalGuard { mouse };
    if mouse {
        execute!(std::io::stdout(), EnableMouseCapture)?;
    }

    let mut app = App::new(
        make_sampler(cli, options),
        AppOptions {
            interval_ms,
            history_secs,
            window_secs: 60,
            mouse_enabled: mouse,
            ascii: cli.ascii,
            raw_path: cli.raw.clone(),
            report_path: cli.report.clone(),
        },
    );
    let stop = install_signal_flag()?;
    spawn_terminal_watchdog(stop.clone());

    let mut last_tick = Instant::now();
    loop {
        if stop.load(Ordering::Relaxed) {
            app.should_quit = true;
            break;
        }
        let tick_rate = Duration::from_millis(app.interval_ms);
        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => app.on_key(key),
                Event::Mouse(mouse) => app.on_mouse(mouse),
                Event::Resize(_, _) => app.needs_redraw = true,
                _ => {}
            }
        }
        if let Some(on) = app.take_mouse_toggle() {
            if on {
                execute!(std::io::stdout(), EnableMouseCapture)?;
            } else {
                execute!(std::io::stdout(), DisableMouseCapture)?;
            }
        }
        if last_tick.elapsed() >= tick_rate {
            app.tick();
            last_tick = Instant::now();
        }
        if app.needs_redraw {
            terminal.draw(|frame| ui::render(frame, &mut app, &theme))?;
            app.needs_redraw = false;
        }
        if app.should_quit {
            break;
        }
    }

    let raw_path = app.recorder.stop().or_else(|| app.last_raw.clone());
    drop(guard);
    println!("{}", app.session_summary());
    if let Some(path) = raw_path {
        println!("raw: {}", path.display());
    }
    if app.report_on_exit.is_some() {
        match app.export_on_exit() {
            Ok(Some(path)) => println!("report: {}", path.display()),
            Ok(None) => {}
            Err(err) => eprintln!("report: {err}"),
        }
    }
    Ok(())
}

fn print_tsv_header() {
    println!("# macpower headless v1");
    println!("# columns: {}", raw::COLUMNS.join("\t"));
    println!("{}", raw::COLUMNS.join("\t"));
}

fn print_snapshot(format: Format, snapshot: &Snapshot) -> Result<()> {
    match format {
        Format::Tsv => println!("{}", raw::encode_row(snapshot)),
        Format::Json => println!("{}", serde_json::to_string(&sample_json(snapshot))?),
    }
    Ok(())
}

fn sample_json(snapshot: &Snapshot) -> serde_json::Value {
    let s = &snapshot.sample;
    let ts_ms =
        s.ts.duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_millis() as u64;
    let iso = DateTime::<Local>::from(s.ts).to_rfc3339_opts(SecondsFormat::Millis, true);
    json!({
        "ts_ms": ts_ms,
        "iso": iso,
        "source": s.source.label(),
        "system_w": s.system_w,
        "smc_pstr_w": s.smc_pstr_w,
        "dc_in_w": s.dc_in_w,
        "battery_w": s.battery_w,
        "battery_w_measured": s.battery_w_measured,
        "battery_a_measured": s.battery_a_measured,
        "battery_estimated": s.battery_estimated,
        "battery_pct": s.battery_pct,
        "battery_v": s.battery_v,
        "battery_a": s.battery_a,
        "battery_temp_c": s.battery_temp_c,
        "cycle_count": s.cycle_count,
        "cpu_w": s.cpu_w,
        "ecpu_w": s.ecpu_w,
        "pcpu_w": s.pcpu_w,
        "core_e_w": s.core_e_w,
        "core_p_w": s.core_p_w,
        "gpu_w": s.gpu_w,
        "ane_w": s.ane_w,
        "dram_w": s.dram_w,
        "display_w": s.display_w,
        "other_w": s.other_w,
        "rails": snapshot.rails.iter().map(|r| json!({
            "name": r.name,
            "group": r.group,
            "w": r.w,
            "raw_unit": r.unit,
            "source": r.source.label(),
        })).collect::<Vec<_>>(),
        "battery": {
            "present": snapshot.battery.present,
            "pct": snapshot.battery.pct,
            "health_pct": snapshot.battery.health_pct,
            "design_capacity_mah": snapshot.battery.design_capacity_mah,
            "max_capacity_mah": snapshot.battery.max_capacity_mah,
            "current_capacity_mah": snapshot.battery.current_capacity_mah,
            "external_connected": snapshot.battery.external_connected,
            "is_charging": snapshot.battery.is_charging,
            "fully_charged": snapshot.battery.fully_charged,
            "time_remaining_min": snapshot.battery.time_remaining_min,
            "adapter_watts": snapshot.battery.adapter_watts,
            "adapter_name": snapshot.battery.adapter_name,
            "cell_voltages_v": snapshot.battery.cell_voltages_v,
            "source": snapshot.battery.source.label(),
        },
        "system": {
            "hostname": snapshot.system.hostname,
            "chip": snapshot.system.chip,
            "model": snapshot.system.model,
            "macos": snapshot.system.os_version,
            "build": snapshot.system.os_build,
            "cores": snapshot.system.physical_cores,
            "memory_bytes": snapshot.system.memory_bytes,
            "uptime_s": snapshot.system.uptime_secs,
        },
    })
}

fn write_single_raw(path: &Path, snapshot: &Snapshot, system: &model::SystemInfo) -> Result<()> {
    let mut recorder = Recorder::new();
    recorder
        .start(path.to_path_buf(), system)
        .with_context(|| format!("cannot create {}", path.display()))?;
    recorder.write(snapshot)?;
    recorder.stop();
    Ok(())
}

fn write_single_report(path: &Path, snapshot: &Snapshot, system: &model::SystemInfo) -> Result<()> {
    let samples = vec![snapshot.sample.clone()];
    let input = ReportInput {
        samples: &samples,
        system,
        rails: &snapshot.rails,
        source_label: "single sample".to_string(),
    };
    write_report(path, &input).map_err(anyhow::Error::msg)?;
    Ok(())
}

fn install_signal_flag() -> Result<Arc<AtomicBool>> {
    let flag = Arc::new(AtomicBool::new(false));
    for signal in [
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGHUP,
    ] {
        signal_hook::flag::register(signal, flag.clone())
            .with_context(|| format!("cannot register signal {signal}"))?;
    }
    Ok(flag)
}

/// crossterm 0.29's mio event source spins forever when the terminal pty
/// reaches EOF (`read` returns 0 but it never breaks out of its loop), which
/// leaves an orphaned TUI pegging one CPU core after its terminal is closed.
/// The main thread can be stuck inside that loop, so a separate thread forces
/// the process down shortly after SIGINT/SIGTERM/SIGHUP.
fn spawn_terminal_watchdog(stop: Arc<AtomicBool>) {
    std::thread::spawn(move || loop {
        if stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(750));
            force_terminal_exit();
        }
        std::thread::sleep(Duration::from_millis(100));
    });
}

fn force_terminal_exit() -> ! {
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    std::process::exit(1);
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(std::io::stdout(), DisableMouseCapture);
        previous(info);
    }));
}

struct TerminalGuard {
    mouse: bool,
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.mouse {
            let _ = execute!(std::io::stdout(), DisableMouseCapture);
        }
        ratatui::restore();
    }
}
