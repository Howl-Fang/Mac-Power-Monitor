//! AppleSmartBattery IORegistry reader with an IOPS fallback.

use core_foundation_sys::array::CFArrayRef;
use core_foundation_sys::base::{kCFAllocatorDefault, CFTypeRef};
use core_foundation_sys::dictionary::{CFDictionaryRef, CFMutableDictionaryRef};

use crate::collectors::ffi::cf::{self, CFGuard};
use crate::collectors::ffi::iokit::{self, IoService, KERN_SUCCESS};
use crate::model::{BatteryInfo, PowerSource};

pub struct BatteryReader {
    service: Option<IoService>,
    enabled: bool,
}

impl BatteryReader {
    pub fn open() -> Self {
        Self {
            service: iokit::open_service("AppleSmartBattery"),
            enabled: true,
        }
    }

    pub fn disabled() -> Self {
        Self {
            service: None,
            enabled: false,
        }
    }

    pub fn read(&mut self) -> BatteryInfo {
        if !self.enabled {
            return BatteryInfo::default();
        }
        let mut info = match self.service {
            Some(service) => read_smart_battery(service).unwrap_or_default(),
            None => BatteryInfo::default(),
        };
        if !info.present {
            if let Some(fallback) = read_iops() {
                info = fallback;
            }
        } else {
            // Fill gaps (time remaining, percent) from IOPS when the registry
            // keys are absent on a particular battery firmware.
            if info.pct.is_none() || info.time_remaining_min.is_none() {
                if let Some(fallback) = read_iops() {
                    if info.pct.is_none() {
                        info.pct = fallback.pct;
                    }
                    if info.time_remaining_min.is_none() {
                        info.time_remaining_min = fallback.time_remaining_min;
                    }
                    if info.time_to_full_min.is_none() {
                        info.time_to_full_min = fallback.time_to_full_min;
                    }
                    if info.source == PowerSource::Unknown {
                        info.source = fallback.source;
                    }
                }
            }
        }
        info
    }
}

impl Drop for BatteryReader {
    fn drop(&mut self) {
        if let Some(service) = self.service {
            unsafe { iokit::IOObjectRelease(service) };
        }
    }
}

fn read_smart_battery(service: IoService) -> Option<BatteryInfo> {
    let mut props: CFMutableDictionaryRef = std::ptr::null_mut();
    let kr = unsafe {
        iokit::IORegistryEntryCreateCFProperties(service, &mut props, kCFAllocatorDefault, 0)
    };
    if kr != KERN_SUCCESS || props.is_null() {
        return None;
    }
    let _guard = CFGuard(props as CFTypeRef);
    let dict: CFDictionaryRef = props;

    let voltage_v = cf::dict_get_i64(dict, "Voltage").map(|mv| mv as f32 / 1000.0);
    let amperage_a = cf::dict_get_i64(dict, "InstantAmperage")
        .or_else(|| cf::dict_get_i64(dict, "Amperage"))
        .map(amperage_a);
    let power_w = match (voltage_v, amperage_a) {
        (Some(v), Some(a)) => Some(v * a),
        _ => None,
    };

    let external_connected = cf::dict_get_bool(dict, "ExternalConnected").unwrap_or(false);
    let is_charging = cf::dict_get_bool(dict, "IsCharging").unwrap_or(false);
    let fully_charged = cf::dict_get_bool(dict, "FullyCharged").unwrap_or(false);

    let current_pct = cf::dict_get_i64(dict, "CurrentCapacity")
        .filter(|v| (0..=100).contains(v))
        .map(|v| v as f32)
        .or_else(|| {
            let current = cf::dict_get_i64(dict, "AppleRawCurrentCapacity")? as f32;
            let max = cf::dict_get_i64(dict, "AppleRawMaxCapacity")? as f32;
            if max > 0.0 {
                Some((current / max * 100.0).clamp(0.0, 100.0))
            } else {
                None
            }
        });

    let design_mah = cf::dict_get_i64(dict, "DesignCapacity")
        .filter(|v| *v > 0)
        .map(|v| v as u32);
    let max_mah = cf::dict_get_i64(dict, "NominalChargeCapacity")
        .filter(|v| *v > 0)
        .or_else(|| cf::dict_get_i64(dict, "AppleRawMaxCapacity").filter(|v| *v > 0))
        .map(|v| v as u32);
    let current_mah = cf::dict_get_i64(dict, "AppleRawCurrentCapacity")
        .filter(|v| *v >= 0)
        .map(|v| v as u32);
    let health_pct = match (max_mah, design_mah) {
        (Some(max), Some(design)) if design > 0 => Some(max as f32 / design as f32 * 100.0),
        _ => None,
    };

    let temp_c = cf::dict_get_i64(dict, "Temperature")
        .filter(|v| *v > 0)
        .map(|v| v as f32 / 10.0 - 273.15);

    let time_remaining_min = cf::dict_get_i64(dict, "TimeRemaining")
        .filter(|v| (0..=10_000).contains(v))
        .map(|v| v as i32)
        .or_else(|| {
            cf::dict_get_i64(dict, "AvgTimeToEmpty")
                .filter(|v| (0..=10_000).contains(v))
                .map(|v| v as i32)
        });
    let time_to_full_min = cf::dict_get_i64(dict, "AvgTimeToFull")
        .filter(|v| (0..=10_000).contains(v))
        .map(|v| v as i32);

    let adapter = cf::dict_get_dict(dict, "AdapterDetails");
    let adapter_watts = adapter
        .and_then(|d| cf::dict_get_f64(d, "Watts"))
        .map(|v| v as f32);
    let adapter_name = adapter.and_then(|d| {
        cf::dict_get_string(d, "Name").or_else(|| cf::dict_get_string(d, "Description"))
    });

    let telemetry = cf::dict_get_dict(dict, "PowerTelemetryData");
    let fw_system_power_in_w = telemetry
        .and_then(|d| cf::dict_get_i64(d, "SystemPowerIn"))
        .and_then(guess_telemetry_watts);
    let fw_system_load_w = telemetry
        .and_then(|d| cf::dict_get_i64(d, "SystemLoad"))
        .and_then(guess_telemetry_watts);

    let cell_voltages_v = cf::dict_get_dict(dict, "BatteryData")
        .and_then(|d| cf::dict_get_array(d, "CellVoltage"))
        .map(|array| {
            let n = cf::array_len(array);
            (0..n)
                .filter_map(|i| cf::array_get_i64(array, i))
                .map(|mv| mv as f32 / 1000.0)
                .collect()
        })
        .unwrap_or_default();

    let source = if external_connected {
        PowerSource::Ac
    } else {
        PowerSource::Battery
    };

    Some(BatteryInfo {
        present: true,
        pct: current_pct,
        voltage_v,
        amperage_a,
        power_w,
        temperature_c: temp_c,
        cycle_count: cf::dict_get_i64(dict, "CycleCount")
            .filter(|v| *v >= 0)
            .map(|v| v as u32),
        design_capacity_mah: design_mah,
        max_capacity_mah: max_mah,
        current_capacity_mah: current_mah,
        health_pct,
        external_connected,
        is_charging,
        fully_charged,
        time_remaining_min,
        time_to_full_min,
        adapter_watts,
        adapter_name,
        fw_system_power_in_w,
        fw_system_load_w,
        cell_voltages_v,
        source,
    })
}

/// Some battery firmwares expose a negative current as a zero-extended
/// 32-bit value. Reinterpret values above i32::MAX through u32 first.
fn amperage_a(raw_ma: i64) -> f32 {
    let ma = if raw_ma > i32::MAX as i64 {
        raw_ma as u32 as i32
    } else {
        raw_ma as i32
    };
    ma as f32 / 1000.0
}

fn guess_telemetry_watts(raw: i64) -> Option<f32> {
    if raw == 0 {
        return Some(0.0);
    }
    let micro = raw as f64 / 1e6;
    let milli = raw as f64 / 1e3;
    for candidate in [milli, micro] {
        if (0.01..=500.0).contains(&candidate) {
            return Some(candidate as f32);
        }
    }
    None
}

fn read_iops() -> Option<BatteryInfo> {
    let blob = unsafe { iokit::IOPSCopyPowerSourcesInfo() };
    if blob.is_null() {
        return None;
    }
    let _blob_guard = CFGuard(blob);
    let list: CFArrayRef = unsafe { iokit::IOPSCopyPowerSourcesList(blob) };
    if list.is_null() {
        return None;
    }
    let _list_guard = CFGuard(list as CFTypeRef);
    let n = cf::array_len(list);
    let mut best: Option<BatteryInfo> = None;
    for i in 0..n {
        let Some(source) = cf::array_get(list, i) else {
            continue;
        };
        let desc = unsafe { iokit::IOPSGetPowerSourceDescription(blob, source) };
        if desc.is_null() {
            continue;
        }
        let state = cf::dict_get_string(desc, "Power Source State").unwrap_or_default();
        let present = cf::dict_get_bool(desc, "Is Present").unwrap_or(true);
        if !present {
            continue;
        }
        let pct = cf::dict_get_i64(desc, "Current Capacity")
            .zip(cf::dict_get_i64(desc, "Max Capacity"))
            .and_then(|(cur, max)| {
                (max > 0).then_some((cur as f32 / max as f32 * 100.0).clamp(0.0, 100.0))
            });
        let time_remaining_min = cf::dict_get_i64(desc, "Time to Empty")
            .filter(|v| (0..=10_000).contains(v))
            .map(|v| v as i32);
        let time_to_full_min = cf::dict_get_i64(desc, "Time to Full Charge")
            .filter(|v| (0..=10_000).contains(v))
            .map(|v| v as i32);
        let is_charging = cf::dict_get_bool(desc, "Is Charging").unwrap_or(false);
        let source_kind = if state.contains("AC") {
            PowerSource::Ac
        } else if state.contains("Battery") {
            PowerSource::Battery
        } else {
            PowerSource::Unknown
        };
        let info = BatteryInfo {
            present: true,
            pct,
            time_remaining_min,
            time_to_full_min,
            is_charging,
            external_connected: source_kind == PowerSource::Ac,
            source: source_kind,
            ..BatteryInfo::default()
        };
        let is_battery = info.source == PowerSource::Battery;
        if is_battery || best.is_none() {
            best = Some(info);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_reader_reports_no_battery() {
        let mut reader = BatteryReader::disabled();
        assert!(!reader.read().present);
    }

    #[test]
    fn normalizes_signed_and_zero_extended_amperage() {
        assert_eq!(amperage_a(2_495), 2.495);
        assert_eq!(amperage_a(-1_500), -1.5);
        assert_eq!(amperage_a(0xFFFF_F448_u32 as i64), -3.0);
        assert_eq!(amperage_a(u32::MAX as i64), -0.001);
    }

    #[test]
    fn telemetry_watt_guess() {
        assert_eq!(guess_telemetry_watts(0), Some(0.0));
        assert_eq!(guess_telemetry_watts(4_723), Some(4.723));
        assert_eq!(guess_telemetry_watts(30_000), Some(30.0));
        assert_eq!(guess_telemetry_watts(30_000_000), Some(30.0));
        assert_eq!(guess_telemetry_watts(i64::MAX), None);
    }
}
