//! Raw TSV recording and parsing.

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Local, SecondsFormat};

use crate::model::{PowerSource, RailValue, Sample, Snapshot, SystemInfo};

pub const COLUMNS: &[&str] = &[
    "timestamp_ms",
    "iso_time",
    "source",
    "system_w",
    "smc_pstr_w",
    "dc_in_w",
    "battery_w",
    "battery_pct",
    "battery_v",
    "battery_a",
    "battery_temp_c",
    "cycle_count",
    "cpu_w",
    "ecpu_w",
    "pcpu_w",
    "core_e_w",
    "core_p_w",
    "gpu_w",
    "ane_w",
    "dram_w",
    "display_w",
    "other_w",
    "rails",
];

pub struct Recorder {
    file: Option<BufWriter<File>>,
    path: Option<PathBuf>,
    last_path: Option<PathBuf>,
    rows: u64,
}

impl Recorder {
    pub fn new() -> Self {
        Self {
            file: None,
            path: None,
            last_path: None,
            rows: 0,
        }
    }

    pub fn is_recording(&self) -> bool {
        self.file.is_some()
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn last_path(&self) -> Option<&Path> {
        self.last_path.as_deref().or(self.path.as_deref())
    }

    pub fn rows(&self) -> u64 {
        self.rows
    }

    pub fn start(&mut self, path: PathBuf, system: &SystemInfo) -> std::io::Result<()> {
        self.stop();
        let file = File::create(&path)?;
        let mut writer = BufWriter::new(file);
        write_header(&mut writer, system)?;
        writer.flush()?;
        self.file = Some(writer);
        self.path = Some(path.clone());
        self.last_path = Some(path);
        self.rows = 0;
        Ok(())
    }

    pub fn write(&mut self, snapshot: &Snapshot) -> std::io::Result<()> {
        let Some(writer) = self.file.as_mut() else {
            return Ok(());
        };
        let line = encode_row(snapshot);
        writeln!(writer, "{line}")?;
        writer.flush()?;
        self.rows += 1;
        Ok(())
    }

    pub fn flush(&mut self) -> std::io::Result<()> {
        if let Some(writer) = self.file.as_mut() {
            writer.flush()?;
        }
        Ok(())
    }

    pub fn stop(&mut self) -> Option<PathBuf> {
        if let Some(mut writer) = self.file.take() {
            let _ = writer.flush();
        }
        self.path.take()
    }
}

impl Default for Recorder {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        let _ = self.flush();
    }
}

fn write_header(writer: &mut impl Write, system: &SystemInfo) -> std::io::Result<()> {
    let start =
        DateTime::<Local>::from(SystemTime::now()).to_rfc3339_opts(SecondsFormat::Millis, true);
    writeln!(writer, "# macpower raw v1")?;
    writeln!(writer, "# start={start}")?;
    writeln!(
        writer,
        "# host={} chip={} model={} macos={} build={}",
        sanitize(&system.hostname),
        sanitize(&system.chip),
        sanitize(&system.model),
        sanitize(&system.os_version),
        sanitize(&system.os_build)
    )?;
    writeln!(writer, "# units: power=W voltage=V current=A temperature=C")?;
    writeln!(writer, "# columns: {}", COLUMNS.join("\t"))?;
    writeln!(writer, "{}", COLUMNS.join("\t"))?;
    Ok(())
}

fn sanitize(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

pub fn encode_row(snapshot: &Snapshot) -> String {
    let s = &snapshot.sample;
    let ts_ms =
        s.ts.duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_millis();
    let iso = DateTime::<Local>::from(s.ts).to_rfc3339_opts(SecondsFormat::Millis, true);
    let fields = [
        ts_ms.to_string(),
        iso,
        s.source.label().to_string(),
        opt_f32(s.system_w),
        opt_f32(s.smc_pstr_w),
        opt_f32(s.dc_in_w),
        opt_f32(s.battery_w),
        opt_f32(s.battery_pct),
        opt_f32(s.battery_v),
        opt_f32(s.battery_a),
        opt_f32(s.battery_temp_c),
        s.cycle_count.map(|v| v.to_string()).unwrap_or_default(),
        opt_f32(s.cpu_w),
        opt_f32(s.ecpu_w),
        opt_f32(s.pcpu_w),
        vec_f32(&s.core_e_w),
        vec_f32(&s.core_p_w),
        opt_f32(s.gpu_w),
        opt_f32(s.ane_w),
        opt_f32(s.dram_w),
        opt_f32(s.display_w),
        opt_f32(s.other_w),
        encode_rails(&snapshot.rails),
    ];
    debug_assert_eq!(fields.len(), COLUMNS.len());
    fields.join("\t")
}

fn opt_f32(value: Option<f32>) -> String {
    match value {
        Some(v) if v.is_finite() => format!("{v:.4}"),
        _ => String::new(),
    }
}

fn vec_f32(values: &[f32]) -> String {
    values
        .iter()
        .filter(|v| v.is_finite())
        .map(|v| format!("{v:.4}"))
        .collect::<Vec<_>>()
        .join(";")
}

fn encode_rails(rails: &[RailValue]) -> String {
    rails
        .iter()
        .filter(|r| is_top_level_rail(&r.name))
        .map(|r| format!("{}={:.4}", r.name.replace([';', '='], "_"), r.w))
        .collect::<Vec<_>>()
        .join(";")
}

pub(crate) fn is_top_level_rail(name: &str) -> bool {
    if name.ends_with("_SRAM") || name.contains("DTL") || name.contains("Power Zone Filter") {
        return false;
    }
    if name == "GPU SRAM" || name == "ECPM" || name == "PCPM" {
        return false;
    }
    true
}

pub fn parse_file(path: &Path) -> Result<Vec<Sample>, String> {
    let file = File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let reader = BufReader::new(file);
    let mut header: Option<Vec<String>> = None;
    let mut out = Vec::new();
    for (lineno, line) in reader.lines().enumerate() {
        let line = line.map_err(|e| format!("read {}: {e}", path.display()))?;
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with('#') {
            if let Some(cols) = line.strip_prefix("# columns:") {
                header = Some(cols.trim().split('\t').map(|s| s.to_string()).collect());
            }
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        let Some(header) = header.as_ref() else {
            return Err(format!(
                "{}:{}: data row without a # columns header",
                path.display(),
                lineno + 1
            ));
        };
        if cols.len() != header.len() {
            continue;
        }
        if cols.iter().zip(header.iter()).all(|(a, b)| a == b) {
            continue;
        }
        if let Some(sample) = parse_row(header, &cols) {
            out.push(sample);
        }
    }
    if out.is_empty() {
        return Err(format!("{}: no samples found", path.display()));
    }
    Ok(out)
}

fn parse_row(header: &[String], cols: &[&str]) -> Option<Sample> {
    let get = |name: &str| {
        header
            .iter()
            .position(|h| h == name)
            .and_then(|i| cols.get(i).copied())
    };

    let ts_ms: i64 = get("timestamp_ms")?.parse().ok()?;
    let ts = if ts_ms >= 0 {
        UNIX_EPOCH + Duration::from_millis(ts_ms as u64)
    } else {
        UNIX_EPOCH
    };
    let source = match get("source").unwrap_or("") {
        "Battery" => PowerSource::Battery,
        "AC" => PowerSource::Ac,
        _ => PowerSource::Unknown,
    };

    Some(Sample {
        ts,
        system_w: parse_opt_f32(get("system_w")),
        smc_pstr_w: parse_opt_f32(get("smc_pstr_w")),
        dc_in_w: parse_opt_f32(get("dc_in_w")),
        battery_w: parse_opt_f32(get("battery_w")),
        battery_pct: parse_opt_f32(get("battery_pct")),
        battery_v: parse_opt_f32(get("battery_v")),
        battery_a: parse_opt_f32(get("battery_a")),
        battery_temp_c: parse_opt_f32(get("battery_temp_c")),
        cycle_count: get("cycle_count").and_then(|v| v.parse().ok()),
        cpu_w: parse_opt_f32(get("cpu_w")),
        ecpu_w: parse_opt_f32(get("ecpu_w")),
        pcpu_w: parse_opt_f32(get("pcpu_w")),
        core_e_w: parse_vec_f32(get("core_e_w").unwrap_or("")),
        core_p_w: parse_vec_f32(get("core_p_w").unwrap_or("")),
        gpu_w: parse_opt_f32(get("gpu_w")),
        ane_w: parse_opt_f32(get("ane_w")),
        dram_w: parse_opt_f32(get("dram_w")),
        display_w: parse_opt_f32(get("display_w")),
        other_w: parse_opt_f32(get("other_w")),
        source,
    })
}

fn parse_opt_f32(value: Option<&str>) -> Option<f32> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    value.parse::<f32>().ok().filter(|v| v.is_finite())
}

fn parse_vec_f32(value: &str) -> Vec<f32> {
    value
        .split(';')
        .filter(|s| !s.trim().is_empty())
        .filter_map(|s| s.trim().parse::<f32>().ok())
        .filter(|v| v.is_finite())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{BatteryInfo, SmcValues};

    fn sample() -> Sample {
        Sample {
            ts: UNIX_EPOCH + Duration::from_millis(1_700_000_000_123),
            system_w: Some(4.72),
            smc_pstr_w: Some(3.85),
            dc_in_w: None,
            battery_w: Some(-4.44),
            battery_pct: Some(57.0),
            battery_v: Some(11.63),
            battery_a: Some(-0.33),
            battery_temp_c: Some(30.2),
            cycle_count: Some(426),
            cpu_w: Some(0.131),
            ecpu_w: Some(0.085),
            pcpu_w: Some(0.047),
            core_e_w: vec![0.015, 0.016],
            core_p_w: vec![0.011],
            gpu_w: Some(0.0),
            ane_w: Some(0.0),
            dram_w: Some(0.059),
            display_w: Some(0.017),
            other_w: Some(0.008),
            source: PowerSource::Battery,
        }
    }

    #[test]
    fn row_round_trip() {
        let snapshot = Snapshot {
            sample: sample(),
            rails: vec![RailValue {
                name: "CPU Energy".to_string(),
                group: "Energy Model".to_string(),
                w: 0.131,
                unit: "mJ".to_string(),
                source: crate::model::RailSource::IoReport,
            }],
            battery: BatteryInfo::default(),
            smc: SmcValues::default(),
            system: SystemInfo::default(),
        };
        let row = encode_row(&snapshot);
        let header: Vec<String> = COLUMNS.iter().map(|s| s.to_string()).collect();
        let cols: Vec<&str> = row.split('\t').collect();
        let parsed = parse_row(&header, &cols).unwrap();
        assert_eq!(parsed.ts, snapshot.sample.ts);
        assert_eq!(parsed.system_w, Some(4.72));
        assert_eq!(parsed.core_e_w, vec![0.015, 0.016]);
        assert_eq!(parsed.source, PowerSource::Battery);
        assert_eq!(parsed.cycle_count, Some(426));
    }

    #[test]
    fn missing_values_are_none() {
        let mut s = sample();
        s.system_w = None;
        s.core_e_w.clear();
        let snapshot = Snapshot {
            sample: s,
            rails: Vec::new(),
            battery: BatteryInfo::default(),
            smc: SmcValues::default(),
            system: SystemInfo::default(),
        };
        let row = encode_row(&snapshot);
        let header: Vec<String> = COLUMNS.iter().map(|s| s.to_string()).collect();
        let cols: Vec<&str> = row.split('\t').collect();
        let parsed = parse_row(&header, &cols).unwrap();
        assert_eq!(parsed.system_w, None);
        assert!(parsed.core_e_w.is_empty());
    }

    #[test]
    fn filters_subchannels_from_rails() {
        assert!(is_top_level_rail("CPU Energy"));
        assert!(is_top_level_rail("ECPU0"));
        assert!(is_top_level_rail("PSTR"));
        assert!(!is_top_level_rail("ECPU0_SRAM"));
        assert!(!is_top_level_rail("PCPUDTL00"));
        assert!(!is_top_level_rail("GPU Power Zone Filter 0"));
        assert!(!is_top_level_rail("ECPM"));
    }
}
