use std::time::SystemTime;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PowerSource {
    Battery,
    Ac,
    Unknown,
}

impl PowerSource {
    pub fn label(self) -> &'static str {
        match self {
            PowerSource::Battery => "Battery",
            PowerSource::Ac => "AC",
            PowerSource::Unknown => "Unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RailSource {
    Smc,
    IoReport,
    IoKit,
    Firmware,
}

impl RailSource {
    pub fn label(self) -> &'static str {
        match self {
            RailSource::Smc => "SMC",
            RailSource::IoReport => "IOReport",
            RailSource::IoKit => "IOKit",
            RailSource::Firmware => "FW",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RailValue {
    pub name: String,
    pub group: String,
    pub w: f32,
    pub unit: String,
    pub source: RailSource,
}

#[derive(Clone, Debug, Default)]
pub struct SmcValues {
    pub pstr_w: Option<f32>,
    pub ppbr_w: Option<f32>,
    pub pdtr_w: Option<f32>,
    pub backlight_w: Option<f32>,
}

#[derive(Clone, Debug)]
pub struct BatteryInfo {
    pub present: bool,
    pub pct: Option<f32>,
    pub voltage_v: Option<f32>,
    pub amperage_a: Option<f32>,
    /// Signed: positive while charging, negative while discharging.
    pub power_w: Option<f32>,
    pub temperature_c: Option<f32>,
    pub cycle_count: Option<u32>,
    pub design_capacity_mah: Option<u32>,
    pub max_capacity_mah: Option<u32>,
    pub current_capacity_mah: Option<u32>,
    pub health_pct: Option<f32>,
    pub external_connected: bool,
    pub is_charging: bool,
    pub fully_charged: bool,
    pub time_remaining_min: Option<i32>,
    pub time_to_full_min: Option<i32>,
    pub adapter_watts: Option<f32>,
    pub adapter_name: Option<String>,
    pub fw_system_power_in_w: Option<f32>,
    pub fw_system_load_w: Option<f32>,
    pub cell_voltages_v: Vec<f32>,
    pub source: PowerSource,
}

impl Default for BatteryInfo {
    fn default() -> Self {
        Self {
            present: false,
            pct: None,
            voltage_v: None,
            amperage_a: None,
            power_w: None,
            temperature_c: None,
            cycle_count: None,
            design_capacity_mah: None,
            max_capacity_mah: None,
            current_capacity_mah: None,
            health_pct: None,
            external_connected: false,
            is_charging: false,
            fully_charged: false,
            time_remaining_min: None,
            time_to_full_min: None,
            adapter_watts: None,
            adapter_name: None,
            fw_system_power_in_w: None,
            fw_system_load_w: None,
            cell_voltages_v: Vec::new(),
            source: PowerSource::Unknown,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SystemInfo {
    pub hostname: String,
    pub chip: String,
    pub model: String,
    pub os_version: String,
    pub os_build: String,
    pub physical_cores: usize,
    pub p_cores: Option<usize>,
    pub e_cores: Option<usize>,
    pub memory_bytes: u64,
    pub uptime_secs: u64,
    pub rosetta: bool,
}

#[derive(Clone, Debug)]
pub struct Sample {
    pub ts: SystemTime,
    pub system_w: Option<f32>,
    pub smc_pstr_w: Option<f32>,
    pub dc_in_w: Option<f32>,
    pub battery_w: Option<f32>,
    pub battery_pct: Option<f32>,
    pub battery_v: Option<f32>,
    pub battery_a: Option<f32>,
    pub battery_temp_c: Option<f32>,
    pub cycle_count: Option<u32>,
    pub cpu_w: Option<f32>,
    pub ecpu_w: Option<f32>,
    pub pcpu_w: Option<f32>,
    pub core_e_w: Vec<f32>,
    pub core_p_w: Vec<f32>,
    pub gpu_w: Option<f32>,
    pub ane_w: Option<f32>,
    pub dram_w: Option<f32>,
    pub display_w: Option<f32>,
    pub other_w: Option<f32>,
    pub source: PowerSource,
}

impl Default for Sample {
    fn default() -> Self {
        Self {
            ts: SystemTime::UNIX_EPOCH,
            system_w: None,
            smc_pstr_w: None,
            dc_in_w: None,
            battery_w: None,
            battery_pct: None,
            battery_v: None,
            battery_a: None,
            battery_temp_c: None,
            cycle_count: None,
            cpu_w: None,
            ecpu_w: None,
            pcpu_w: None,
            core_e_w: Vec::new(),
            core_p_w: Vec::new(),
            gpu_w: None,
            ane_w: None,
            dram_w: None,
            display_w: None,
            other_w: None,
            source: PowerSource::Unknown,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub sample: Sample,
    pub rails: Vec<RailValue>,
    pub battery: BatteryInfo,
    pub smc: SmcValues,
    pub system: SystemInfo,
}

pub fn fmt_power(w: Option<f32>) -> String {
    match w {
        Some(v) => fmt_power_val(v),
        None => "n/a".to_string(),
    }
}

pub fn fmt_power_val(w: f32) -> String {
    if !w.is_finite() {
        return "n/a".to_string();
    }
    if w.abs() < 1.0 {
        format!("{:.0} mW", w * 1000.0)
    } else {
        format!("{:.2} W", w)
    }
}

pub fn fmt_pct(pct: Option<f32>) -> String {
    match pct {
        Some(v) if v.is_finite() => format!("{:.0}%", v),
        _ => "n/a".to_string(),
    }
}

pub fn fmt_voltage(v: Option<f32>) -> String {
    match v {
        Some(v) if v.is_finite() => format!("{:.2} V", v),
        _ => "n/a".to_string(),
    }
}

pub fn fmt_current(a: Option<f32>) -> String {
    match a {
        Some(v) if v.is_finite() => format!("{:.2} A", v),
        _ => "n/a".to_string(),
    }
}

pub fn fmt_temp(c: Option<f32>) -> String {
    match c {
        Some(v) if v.is_finite() => format!("{:.1} °C", v),
        _ => "n/a".to_string(),
    }
}

pub fn fmt_duration_min(min: Option<i32>) -> String {
    let Some(min) = min else {
        return "n/a".to_string();
    };
    if min < 0 {
        return "n/a".to_string();
    }
    let h = min / 60;
    let m = min % 60;
    if h >= 24 {
        format!("{}d {:02}:{:02}", h / 24, h % 24, m)
    } else {
        format!("{}:{:02}", h, m)
    }
}

pub fn fmt_duration_secs(secs: u64) -> String {
    let d = secs / 86_400;
    let h = (secs % 86_400) / 3_600;
    let m = (secs % 3_600) / 60;
    let s = secs % 60;
    if d > 0 {
        format!("{}d {:02}:{:02}:{:02}", d, h, m, s)
    } else if h > 0 {
        format!("{}:{:02}:{:02}", h, m, s)
    } else {
        format!("{}:{:02}", m, s)
    }
}

pub fn fmt_wh(wh: f64) -> String {
    if !wh.is_finite() {
        return "n/a".to_string();
    }
    if wh.abs() < 1.0 {
        format!("{:.1} mWh", wh * 1000.0)
    } else {
        format!("{:.2} Wh", wh)
    }
}

pub fn fmt_memory(bytes: u64) -> String {
    let gb = bytes as f64 / 1_073_741_824.0;
    format!("{:.0} GB", gb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_small_and_large_power() {
        assert_eq!(fmt_power_val(0.015), "15 mW");
        assert_eq!(fmt_power_val(4.72), "4.72 W");
        assert_eq!(fmt_power_val(-4.44), "-4.44 W");
        assert_eq!(fmt_power(None), "n/a");
    }

    #[test]
    fn formats_duration() {
        assert_eq!(fmt_duration_min(Some(162)), "2:42");
        assert_eq!(fmt_duration_min(Some(-1)), "n/a");
        assert_eq!(fmt_duration_min(Some(1500)), "1d 01:00");
    }
}
