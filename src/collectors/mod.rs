pub mod battery;
pub mod ffi;
pub mod ioreport;
pub mod smc;
pub mod sysinfo;

use std::time::{Instant, SystemTime};

use crate::model::{
    fmt_power, fmt_voltage, BatteryInfo, PowerSource, RailSource, RailValue, Sample, SmcValues,
    Snapshot, SystemInfo,
};

use self::battery::BatteryReader;
use self::ioreport::{ComponentPower, IoReportClient};
use self::smc::SmcClient;

#[derive(Clone, Copy, Debug, Default)]
pub struct CollectorOptions {
    pub disable_smc: bool,
    pub disable_ioreport: bool,
    pub disable_battery: bool,
}

pub enum Sampler {
    Real(Box<RealSampler>),
    Demo(DemoSampler),
}

impl Sampler {
    pub fn real(options: CollectorOptions) -> Self {
        Self::Real(Box::new(RealSampler::new(options)))
    }

    pub fn demo() -> Self {
        Self::Demo(DemoSampler::new())
    }

    pub fn tick(&mut self) -> Snapshot {
        match self {
            Sampler::Real(s) => s.tick(),
            Sampler::Demo(s) => s.tick(),
        }
    }

    pub fn reset(&mut self) {
        match self {
            Sampler::Real(s) => s.reset(),
            Sampler::Demo(_) => {}
        }
    }

    pub fn warnings(&self) -> &[String] {
        match self {
            Sampler::Real(s) => &s.warnings,
            Sampler::Demo(_) => &[],
        }
    }

    pub fn system(&self) -> &SystemInfo {
        match self {
            Sampler::Real(s) => &s.system,
            Sampler::Demo(s) => &s.system,
        }
    }

    pub fn sensor_report(&mut self) -> String {
        match self {
            Sampler::Real(s) => s.sensor_report(),
            Sampler::Demo(s) => s.sensor_report(),
        }
    }
}

pub struct RealSampler {
    smc: Option<SmcClient>,
    ioreport: Option<IoReportClient>,
    battery: BatteryReader,
    battery_estimator: BatteryPowerEstimator,
    system: SystemInfo,
    warnings: Vec<String>,
    smc_disabled: bool,
    ioreport_disabled: bool,
    battery_disabled: bool,
}

#[derive(Default)]
struct BatteryPowerEstimator {
    last_measured_w: Option<f32>,
    charge_ratio: Option<f32>,
    last_external: Option<bool>,
    last_charging: Option<bool>,
}

#[derive(Clone, Copy, Debug)]
struct BatteryEstimate {
    power_w: Option<f32>,
    amperage_a: Option<f32>,
    estimated: bool,
}

impl BatteryPowerEstimator {
    fn estimate(&mut self, smc: &SmcValues, battery: &BatteryInfo) -> BatteryEstimate {
        let measured = battery.power_w;
        let state_changed = self.last_external != Some(battery.external_connected)
            || self.last_charging != Some(battery.is_charging);
        if state_changed {
            self.charge_ratio = None;
            self.last_measured_w = None;
        }
        self.last_external = Some(battery.external_connected);
        self.last_charging = Some(battery.is_charging);
        let fresh = match (measured, self.last_measured_w) {
            (Some(now), Some(prev)) => (now - prev).abs() > 0.02,
            (Some(_), None) => true,
            _ => false,
        };

        let estimate = if !battery.present {
            None
        } else if !battery.external_connected {
            smc.ppbr_w
                .filter(|w| w.is_finite() && *w > 0.05)
                .map(|w| self.derived(-w, battery))
        } else {
            match (smc.pdtr_w, smc.pstr_w) {
                (Some(pdtr), Some(pstr)) => {
                    let raw = pdtr - pstr;
                    // macOS can keep IsCharging false for many seconds after
                    // the adapter is connected. If the adapter clearly
                    // supplies more than the system uses and the pack is not
                    // full, the difference is charging the battery.
                    let charging = battery.is_charging || (!battery.fully_charged && raw > 3.0);
                    if charging && raw > 0.05 {
                        if fresh {
                            if let Some(power) = measured.filter(|w| *w > 0.05) {
                                self.charge_ratio = Some((power / raw).clamp(0.5, 1.2));
                            }
                        }
                        Some(self.derived(raw * self.charge_ratio.unwrap_or(0.9), battery))
                    } else {
                        None
                    }
                }
                _ => None,
            }
        };

        self.last_measured_w = measured;
        estimate.unwrap_or(BatteryEstimate {
            power_w: measured,
            amperage_a: battery.amperage_a,
            estimated: false,
        })
    }

    fn derived(&self, power_w: f32, battery: &BatteryInfo) -> BatteryEstimate {
        let amperage_a = battery
            .voltage_v
            .filter(|v| v.is_finite() && *v > 0.1)
            .map(|v| power_w / v)
            .or(battery.amperage_a);
        BatteryEstimate {
            power_w: Some(power_w),
            amperage_a,
            estimated: true,
        }
    }
}

impl RealSampler {
    pub fn new(options: CollectorOptions) -> Self {
        let system = SystemInfo::detect();
        let mut warnings = Vec::new();

        let smc = if options.disable_smc {
            None
        } else {
            match SmcClient::open() {
                Ok(client) => Some(client),
                Err(err) => {
                    warnings.push(format!("SMC unavailable: {err}"));
                    None
                }
            }
        };

        let ioreport = if options.disable_ioreport {
            None
        } else {
            match IoReportClient::open() {
                Ok(client) => Some(client),
                Err(err) => {
                    warnings.push(format!("IOReport unavailable: {err}"));
                    None
                }
            }
        };

        let battery = if options.disable_battery {
            BatteryReader::disabled()
        } else {
            BatteryReader::open()
        };

        Self {
            smc,
            ioreport,
            battery,
            battery_estimator: BatteryPowerEstimator::default(),
            system,
            warnings,
            smc_disabled: options.disable_smc,
            ioreport_disabled: options.disable_ioreport,
            battery_disabled: options.disable_battery,
        }
    }

    fn tick(&mut self) -> Snapshot {
        let smc_values = self
            .smc
            .as_mut()
            .map(|smc| smc.read_power())
            .unwrap_or_default();
        let battery = self.battery.read();
        let rails = self
            .ioreport
            .as_mut()
            .map(|io| io.sample())
            .unwrap_or_default();
        let comp = ComponentPower::from_rails(&rails);
        let display_w = display_power(comp.display_w, smc_values.backlight_w);

        let source = if battery.present {
            battery.source
        } else if smc_values.pstr_w.is_some() || smc_values.pdtr_w.is_some() {
            PowerSource::Ac
        } else {
            PowerSource::Unknown
        };

        let battery_estimate = self.battery_estimator.estimate(&smc_values, &battery);
        let system_w = pick_system_power(&smc_values, &battery, source, battery_estimate.power_w);
        let dc_in_w = pick_input(&smc_values, &battery);

        let sample = Sample {
            ts: SystemTime::now(),
            system_w,
            smc_pstr_w: smc_values.pstr_w,
            dc_in_w,
            battery_w: battery_estimate.power_w,
            battery_pct: battery.pct,
            battery_v: battery.voltage_v,
            battery_a: battery_estimate.amperage_a,
            battery_w_measured: battery.power_w,
            battery_a_measured: battery.amperage_a,
            battery_estimated: battery_estimate.estimated,
            battery_temp_c: battery.temperature_c,
            cycle_count: battery.cycle_count,
            cpu_w: comp.cpu_w,
            ecpu_w: comp.ecpu_w,
            pcpu_w: comp.pcpu_w,
            core_e_w: comp.core_e_w.clone(),
            core_p_w: comp.core_p_w.clone(),
            gpu_w: comp.gpu_w,
            ane_w: comp.ane_w,
            dram_w: comp.dram_w,
            display_w,
            other_w: comp.other_w,
            source,
        };

        let mut all_rails = rails;
        add_smc_rails(&mut all_rails, &smc_values);
        add_battery_rails(&mut all_rails, &battery);
        add_firmware_rails(&mut all_rails, &battery);

        Snapshot {
            sample,
            rails: all_rails,
            battery,
            smc: smc_values,
            system: self.system.clone(),
        }
    }

    fn reset(&mut self) {
        if let Some(io) = self.ioreport.as_mut() {
            io.reset();
        }
    }

    fn sensor_report(&mut self) -> String {
        let mut out = String::new();
        out.push_str("SMC\n");
        if self.smc_disabled {
            out.push_str("  disabled by --disable smc\n");
        } else if let Some(smc) = self.smc.as_mut() {
            for probe in smc.probe() {
                if probe.available {
                    let value = probe
                        .value
                        .map(|v| fmt_power(Some(v)))
                        .unwrap_or_else(|| "no value".to_string());
                    out.push_str(&format!(
                        "  {:<4} {:<4} {}\n",
                        probe.key, probe.type_name, value
                    ));
                } else {
                    out.push_str(&format!("  {:<4} not present\n", probe.key));
                }
            }
        } else {
            out.push_str("  unavailable\n");
        }

        out.push_str("\nIOReport\n");
        if self.ioreport_disabled {
            out.push_str("  disabled by --disable ioreport\n");
        } else if let Some(io) = self.ioreport.as_mut() {
            let rails = io.probe();
            if rails.is_empty() {
                out.push_str("  no Energy Model channels\n");
            } else {
                out.push_str(&format!("  Energy Model: {} channels\n", rails.len()));
                for rail in rails {
                    out.push_str(&format!(
                        "  {:<22} {:<4} {}\n",
                        rail.name,
                        rail.unit,
                        fmt_power(Some(rail.w))
                    ));
                }
            }
        } else {
            out.push_str("  unavailable\n");
        }

        out.push_str("\nBattery (AppleSmartBattery / IOPS)\n");
        if self.battery_disabled {
            out.push_str("  disabled by --disable battery\n");
        } else {
            let b = self.battery.read();
            if !b.present {
                out.push_str("  no battery (desktop or unavailable)\n");
            } else {
                out.push_str(&format!("  source: {}\n", b.source.label()));
                out.push_str(&format!(
                    "  percent: {}\n",
                    b.pct
                        .map(|v| format!("{v:.0}%"))
                        .unwrap_or_else(|| "n/a".into())
                ));
                out.push_str(&format!("  voltage: {}\n", fmt_voltage(b.voltage_v)));
                out.push_str(&format!(
                    "  current: {}\n",
                    b.amperage_a
                        .map(|v| format!("{v:.3} A"))
                        .unwrap_or_else(|| "n/a".into())
                ));
                out.push_str(&format!("  power: {}\n", fmt_power(b.power_w)));
                out.push_str(&format!(
                    "  temperature: {}\n",
                    b.temperature_c
                        .map(|v| format!("{v:.1} C"))
                        .unwrap_or_else(|| "n/a".into())
                ));
                out.push_str(&format!(
                    "  cycle count: {}\n",
                    b.cycle_count
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "n/a".into())
                ));
                out.push_str(&format!(
                    "  health: {}\n",
                    b.health_pct
                        .map(|v| format!("{v:.0}%"))
                        .unwrap_or_else(|| "n/a".into())
                ));
                if let Some(w) = b.adapter_watts {
                    out.push_str(&format!("  adapter rated: {w:.0} W\n"));
                }
            }
        }
        out
    }
}

pub struct DemoSampler {
    start: Instant,
    system: SystemInfo,
}

impl DemoSampler {
    pub fn new() -> Self {
        let mut system = SystemInfo::detect();
        system.chip = "Demo SoC".to_string();
        system.os_version = "demo".to_string();
        system.hostname = "demo".to_string();
        system.p_cores = Some(4);
        system.e_cores = Some(4);
        system.physical_cores = 8;
        Self {
            start: Instant::now(),
            system,
        }
    }

    fn tick(&mut self) -> Snapshot {
        let t = self.start.elapsed().as_secs_f64();
        let wave = |period: f64, phase: f64| ((t / period + phase) * std::f64::consts::TAU).sin();

        let cpu = (0.13 + 0.06 * wave(7.0, 0.0)).max(0.01) as f32;
        let ecpu = cpu * 0.64;
        let pcpu = cpu * 0.36;
        let core_e: Vec<f32> = (0..4)
            .map(|i| (ecpu / 4.0 * (0.7 + 0.6 * ((i as f32 + 1.0) / 5.0))).max(0.0))
            .collect();
        let core_p: Vec<f32> = (0..4)
            .map(|i| (pcpu / 4.0 * (0.5 + 0.8 * ((i as f32 + 2.0) / 6.0))).max(0.0))
            .collect();
        let gpu = (0.02 + 0.03 * wave(9.0, 0.3)).max(0.0) as f32;
        let ane = 0.0f32;
        let dram = (0.06 + 0.02 * wave(5.0, 0.7)).max(0.0) as f32;
        let display = 0.018f32;
        let other = 0.08f32;
        let system_w = cpu + gpu + ane + dram + display + other;
        let battery_pct = (78.0 - t / 120.0).clamp(5.0, 100.0) as f32;
        let voltage = 11.8f32;
        let current = -(system_w * 1.08 / voltage);
        let battery_w = voltage * current;
        let smc = SmcValues {
            pstr_w: Some(system_w),
            ppbr_w: Some(-battery_w),
            pdtr_w: Some(0.0),
            backlight_w: Some(display),
        };
        let battery = BatteryInfo {
            present: true,
            pct: Some(battery_pct),
            voltage_v: Some(voltage),
            amperage_a: Some(current),
            power_w: Some(battery_w),
            temperature_c: Some(30.0),
            cycle_count: Some(123),
            design_capacity_mah: Some(6000),
            max_capacity_mah: Some(5400),
            current_capacity_mah: Some((battery_pct / 100.0 * 5400.0) as u32),
            health_pct: Some(90.0),
            external_connected: false,
            is_charging: false,
            fully_charged: false,
            time_remaining_min: Some((battery_pct / 100.0 * 300.0) as i32),
            time_to_full_min: None,
            adapter_watts: None,
            adapter_name: None,
            fw_system_power_in_w: None,
            fw_system_load_w: Some(system_w),
            cell_voltages_v: vec![3.93, 3.94, 3.93],
            source: PowerSource::Battery,
        };

        let mut rails = vec![
            RailValue {
                name: "CPU Energy".into(),
                group: "Energy Model".into(),
                w: cpu,
                unit: "mJ".into(),
                source: RailSource::IoReport,
            },
            RailValue {
                name: "ECPU".into(),
                group: "Energy Model".into(),
                w: ecpu,
                unit: "mJ".into(),
                source: RailSource::IoReport,
            },
            RailValue {
                name: "PCPU".into(),
                group: "Energy Model".into(),
                w: pcpu,
                unit: "mJ".into(),
                source: RailSource::IoReport,
            },
            RailValue {
                name: "GPU".into(),
                group: "Energy Model".into(),
                w: gpu,
                unit: "mJ".into(),
                source: RailSource::IoReport,
            },
            RailValue {
                name: "ANE".into(),
                group: "Energy Model".into(),
                w: ane,
                unit: "mJ".into(),
                source: RailSource::IoReport,
            },
            RailValue {
                name: "DRAM".into(),
                group: "Energy Model".into(),
                w: dram,
                unit: "mJ".into(),
                source: RailSource::IoReport,
            },
            RailValue {
                name: "DISP".into(),
                group: "Energy Model".into(),
                w: display,
                unit: "mJ".into(),
                source: RailSource::IoReport,
            },
        ];
        for (i, w) in core_e.iter().enumerate() {
            rails.push(RailValue {
                name: format!("ECPU{i}"),
                group: "Energy Model".into(),
                w: *w,
                unit: "mJ".into(),
                source: RailSource::IoReport,
            });
        }
        for (i, w) in core_p.iter().enumerate() {
            rails.push(RailValue {
                name: format!("PCPU{i}"),
                group: "Energy Model".into(),
                w: *w,
                unit: "mJ".into(),
                source: RailSource::IoReport,
            });
        }

        let sample = Sample {
            ts: SystemTime::now(),
            system_w: Some(system_w),
            smc_pstr_w: Some(system_w),
            dc_in_w: Some(0.0),
            battery_w: Some(battery_w),
            battery_pct: Some(battery_pct),
            battery_v: Some(voltage),
            battery_a: Some(current),
            battery_w_measured: Some(battery_w),
            battery_a_measured: Some(current),
            battery_estimated: false,
            battery_temp_c: Some(30.0),
            cycle_count: Some(123),
            cpu_w: Some(cpu),
            ecpu_w: Some(ecpu),
            pcpu_w: Some(pcpu),
            core_e_w: core_e,
            core_p_w: core_p,
            gpu_w: Some(gpu),
            ane_w: Some(ane),
            dram_w: Some(dram),
            display_w: Some(display),
            other_w: Some(other),
            source: PowerSource::Battery,
        };

        let mut all_rails = rails;
        add_smc_rails(&mut all_rails, &smc);
        add_battery_rails(&mut all_rails, &battery);
        add_firmware_rails(&mut all_rails, &battery);

        Snapshot {
            sample,
            rails: all_rails,
            battery,
            smc,
            system: self.system.clone(),
        }
    }

    fn sensor_report(&mut self) -> String {
        "SMC\n  demo: PSTR/PPBR/PDTR synthetic\n\nIOReport\n  Energy Model: synthetic channels\n\nBattery\n  demo battery\n".to_string()
    }
}

fn display_power(controller_w: Option<f32>, backlight_w: Option<f32>) -> Option<f32> {
    match (
        controller_w,
        backlight_w.filter(|w| w.is_finite() && *w >= 0.0),
    ) {
        (Some(controller), Some(backlight)) => Some(controller + backlight),
        (Some(controller), None) => Some(controller),
        (None, Some(backlight)) => Some(backlight),
        (None, None) => None,
    }
}

fn pick_system_power(
    smc: &SmcValues,
    battery: &BatteryInfo,
    source: PowerSource,
    battery_w: Option<f32>,
) -> Option<f32> {
    if source == PowerSource::Battery {
        return battery_w
            .map(f32::abs)
            .filter(|w| *w > 0.0)
            .or_else(|| smc.pstr_w.filter(|w| *w > 0.0))
            .or_else(|| battery.fw_system_load_w.filter(|w| *w > 0.0));
    }
    smc.pstr_w
        .filter(|w| *w > 0.0)
        .or_else(|| smc.pdtr_w.filter(|w| *w > 0.0))
        .or_else(|| battery.fw_system_load_w.filter(|w| *w > 0.0))
}

fn pick_input(smc: &SmcValues, battery: &BatteryInfo) -> Option<f32> {
    // PDTR can briefly report hundreds of watts while the adapter is
    // disconnected or negotiating. On battery there is no DC input at all.
    if battery.present && !battery.external_connected {
        return None;
    }
    smc.pdtr_w
        .filter(|w| *w > 0.0 && plausible_input_w(*w, battery))
        .or_else(|| {
            battery
                .fw_system_power_in_w
                .filter(|w| *w > 0.0 && plausible_input_w(*w, battery))
        })
        .or_else(|| {
            if battery.is_charging {
                battery.power_w.filter(|w| *w > 0.0)
            } else {
                None
            }
        })
}

fn plausible_input_w(w: f32, battery: &BatteryInfo) -> bool {
    if !w.is_finite() || w < 0.0 {
        return false;
    }
    if let Some(rated) = battery
        .adapter_watts
        .filter(|rated| rated.is_finite() && *rated > 0.0)
    {
        // Allow adapter/telemetry margin, but reject disconnected or
        // mis-scaled readings far beyond what the adapter can deliver.
        return w <= rated * 1.5 + 10.0;
    }
    w <= 1000.0
}

fn add_smc_rails(rails: &mut Vec<RailValue>, smc: &SmcValues) {
    for (name, value) in [
        ("PSTR", smc.pstr_w),
        ("PPBR", smc.ppbr_w),
        ("PDTR", smc.pdtr_w),
        ("Backlight (SMC)", smc.backlight_w),
    ] {
        if let Some(w) = value {
            rails.push(RailValue {
                name: name.to_string(),
                group: "SMC".to_string(),
                w,
                unit: "W".to_string(),
                source: RailSource::Smc,
            });
        }
    }
}

fn add_battery_rails(rails: &mut Vec<RailValue>, battery: &BatteryInfo) {
    if let Some(w) = battery.power_w {
        rails.push(RailValue {
            name: "Battery power (signed)".to_string(),
            group: "Battery".to_string(),
            w,
            unit: "W".to_string(),
            source: RailSource::IoKit,
        });
    }
    if let Some(w) = battery.adapter_watts {
        rails.push(RailValue {
            name: "Adapter rated".to_string(),
            group: "Battery".to_string(),
            w,
            unit: "W".to_string(),
            source: RailSource::IoKit,
        });
    }
}

fn add_firmware_rails(rails: &mut Vec<RailValue>, battery: &BatteryInfo) {
    if let Some(w) = battery.fw_system_power_in_w {
        rails.push(RailValue {
            name: "FW SystemPowerIn".to_string(),
            group: "Firmware".to_string(),
            w,
            unit: "W".to_string(),
            source: RailSource::Firmware,
        });
    }
    if let Some(w) = battery.fw_system_load_w {
        rails.push(RailValue {
            name: "FW SystemLoad".to_string(),
            group: "Firmware".to_string(),
            w,
            unit: "W".to_string(),
            source: RailSource::Firmware,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_sampler_produces_consistent_values() {
        let mut sampler = Sampler::demo();
        let snapshot = sampler.tick();
        assert!(snapshot.sample.system_w.unwrap() > 0.0);
        assert!(snapshot.sample.cpu_w.unwrap() > 0.0);
        assert_eq!(snapshot.sample.core_e_w.len(), 4);
        assert_eq!(snapshot.sample.core_p_w.len(), 4);
        assert!(snapshot.rails.iter().any(|r| r.name == "PSTR"));
    }

    #[test]
    fn estimates_discharge_power_from_ppbr() {
        let smc = SmcValues {
            pstr_w: None,
            ppbr_w: Some(5.0),
            pdtr_w: None,
            backlight_w: None,
        };
        let battery = BatteryInfo {
            present: true,
            power_w: Some(-4.5),
            voltage_v: Some(11.8),
            external_connected: false,
            is_charging: false,
            ..BatteryInfo::default()
        };
        let mut estimator = BatteryPowerEstimator::default();
        let estimate = estimator.estimate(&smc, &battery);
        assert!((estimate.power_w.unwrap() + 5.0).abs() < 1e-6);
        assert!((estimate.amperage_a.unwrap() + 5.0 / 11.8).abs() < 1e-5);
        assert!(estimate.estimated);
    }

    #[test]
    fn estimates_charge_power_from_dc_minus_system() {
        let smc = SmcValues {
            pstr_w: Some(39.0),
            ppbr_w: Some(1.6),
            pdtr_w: Some(64.3),
            backlight_w: None,
        };
        let battery = BatteryInfo {
            present: true,
            power_w: Some(23.0),
            voltage_v: Some(11.8),
            external_connected: true,
            is_charging: true,
            ..BatteryInfo::default()
        };
        let mut estimator = BatteryPowerEstimator::default();
        let estimate = estimator.estimate(&smc, &battery);
        let power = estimate.power_w.unwrap();
        assert!((power - 23.0).abs() < 0.1, "power was {power}");
        assert!(estimate.estimated);
    }

    #[test]
    fn estimates_charging_before_is_charging_flag() {
        let smc = SmcValues {
            pstr_w: Some(8.0),
            ppbr_w: Some(1.2),
            pdtr_w: Some(64.0),
            backlight_w: None,
        };
        // The gauge still shows stale discharge and macOS has not set
        // IsCharging yet, but the adapter is clearly charging the pack.
        let battery = BatteryInfo {
            present: true,
            power_w: Some(-5.9),
            voltage_v: Some(11.8),
            external_connected: true,
            is_charging: false,
            fully_charged: false,
            ..BatteryInfo::default()
        };
        let mut estimator = BatteryPowerEstimator::default();
        let estimate = estimator.estimate(&smc, &battery);
        let power = estimate.power_w.unwrap();
        assert!((power - 56.0 * 0.9).abs() < 0.01, "power was {power}");
        assert!(estimate.estimated);
    }

    #[test]
    fn does_not_infer_charging_on_a_full_battery() {
        let smc = SmcValues {
            pstr_w: Some(8.0),
            ppbr_w: None,
            pdtr_w: Some(64.0),
            backlight_w: None,
        };
        let battery = BatteryInfo {
            present: true,
            power_w: Some(0.0),
            voltage_v: Some(11.8),
            external_connected: true,
            is_charging: false,
            fully_charged: true,
            ..BatteryInfo::default()
        };
        let mut estimator = BatteryPowerEstimator::default();
        let estimate = estimator.estimate(&smc, &battery);
        assert_eq!(estimate.power_w, Some(0.0));
        assert!(!estimate.estimated);
    }

    #[test]
    fn small_adapter_delta_is_not_treated_as_charging() {
        let smc = SmcValues {
            pstr_w: Some(8.0),
            ppbr_w: None,
            pdtr_w: Some(9.5),
            backlight_w: None,
        };
        let battery = BatteryInfo {
            present: true,
            power_w: Some(-5.9),
            voltage_v: Some(11.8),
            external_connected: true,
            is_charging: false,
            fully_charged: false,
            ..BatteryInfo::default()
        };
        let mut estimator = BatteryPowerEstimator::default();
        let estimate = estimator.estimate(&smc, &battery);
        assert_eq!(estimate.power_w, Some(-5.9));
        assert!(!estimate.estimated);
    }

    #[test]
    fn battery_estimate_falls_back_to_measured_gauge() {
        let smc = SmcValues::default();
        let battery = BatteryInfo {
            present: true,
            power_w: Some(-4.5),
            amperage_a: Some(-0.38),
            external_connected: false,
            ..BatteryInfo::default()
        };
        let mut estimator = BatteryPowerEstimator::default();
        let estimate = estimator.estimate(&smc, &battery);
        assert_eq!(estimate.power_w, Some(-4.5));
        assert_eq!(estimate.amperage_a, Some(-0.38));
        assert!(!estimate.estimated);
    }

    #[test]
    fn system_power_falls_back_to_firmware_load() {
        let smc = SmcValues::default();
        let battery = BatteryInfo {
            present: true,
            external_connected: true,
            fw_system_load_w: Some(12.5),
            ..BatteryInfo::default()
        };
        assert_eq!(
            pick_system_power(&smc, &battery, PowerSource::Ac, None),
            Some(12.5)
        );
    }

    #[test]
    fn input_prefers_firmware_telemetry_over_charging_power() {
        let smc = SmcValues::default();
        let battery = BatteryInfo {
            present: true,
            external_connected: true,
            is_charging: true,
            power_w: Some(23.0),
            fw_system_power_in_w: Some(64.0),
            ..BatteryInfo::default()
        };
        assert_eq!(pick_input(&smc, &battery), Some(64.0));
    }

    #[test]
    fn input_is_none_on_battery_even_when_pdtr_spikes() {
        let smc = SmcValues {
            pdtr_w: Some(479.6),
            ..SmcValues::default()
        };
        let battery = BatteryInfo {
            present: true,
            external_connected: false,
            ..BatteryInfo::default()
        };
        assert_eq!(pick_input(&smc, &battery), None);
    }

    #[test]
    fn input_rejects_values_above_the_adapter_rating() {
        let smc = SmcValues {
            pdtr_w: Some(173.7),
            ..SmcValues::default()
        };
        let battery = BatteryInfo {
            present: true,
            external_connected: true,
            adapter_watts: Some(65.0),
            ..BatteryInfo::default()
        };
        assert_eq!(pick_input(&smc, &battery), None);

        let normal = SmcValues {
            pdtr_w: Some(64.3),
            ..SmcValues::default()
        };
        assert_eq!(pick_input(&normal, &battery), Some(64.3));
    }

    #[test]
    fn display_power_adds_backlight_rail() {
        assert_eq!(display_power(Some(0.08), Some(2.5)), Some(2.58));
        assert_eq!(display_power(Some(0.08), None), Some(0.08));
        assert_eq!(display_power(None, Some(2.5)), Some(2.5));
        assert_eq!(display_power(None, Some(f32::NAN)), None);
    }

    #[test]
    fn system_power_prefers_pstr_on_ac() {
        let smc = SmcValues {
            pstr_w: Some(3.8),
            ppbr_w: Some(4.5),
            pdtr_w: None,
            backlight_w: None,
        };
        let battery = BatteryInfo {
            present: true,
            power_w: Some(-4.5),
            source: PowerSource::Battery,
            ..BatteryInfo::default()
        };
        assert_eq!(
            pick_system_power(&smc, &battery, PowerSource::Ac, Some(-4.5)),
            Some(3.8)
        );
    }

    #[test]
    fn system_power_uses_battery_discharge_when_unplugged() {
        let smc = SmcValues {
            pstr_w: Some(3.8),
            ppbr_w: Some(5.5),
            pdtr_w: None,
            backlight_w: None,
        };
        let battery = BatteryInfo {
            present: true,
            power_w: Some(-5.5),
            source: PowerSource::Battery,
            ..BatteryInfo::default()
        };
        assert_eq!(
            pick_system_power(&smc, &battery, PowerSource::Battery, Some(-5.5)),
            Some(5.5)
        );
    }
}
