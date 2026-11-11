//! Private IOReport access to the `Energy Model` group.
//!
//! IOReport has no public headers. The call sequence here mirrors the working C
//! probe: copy channels, subscribe, take samples, diff two samples, then read the
//! per-channel simple integer values and unit labels.

use std::ffi::c_void;
use std::time::{Duration, Instant};

use core_foundation_sys::array::{CFArrayGetCount, CFArrayGetValueAtIndex};
use core_foundation_sys::base::{CFRelease, CFTypeRef};
use core_foundation_sys::dictionary::{CFDictionaryRef, CFMutableDictionaryRef};
use core_foundation_sys::string::CFStringRef;
use libloading::{Library, Symbol};

use crate::collectors::ffi::cf::{self, CFGuard};
use crate::model::{RailSource, RailValue};

type CopyChannelsFn =
    unsafe extern "C" fn(CFStringRef, CFStringRef, u64, u64, u64) -> CFMutableDictionaryRef;
type CreateSubscriptionFn = unsafe extern "C" fn(
    *mut c_void,
    CFMutableDictionaryRef,
    *mut CFMutableDictionaryRef,
    *mut u64,
    CFTypeRef,
) -> *mut c_void;
type CreateSamplesFn =
    unsafe extern "C" fn(*mut c_void, CFMutableDictionaryRef, CFTypeRef) -> CFDictionaryRef;
type CreateSamplesDeltaFn =
    unsafe extern "C" fn(CFDictionaryRef, CFDictionaryRef, CFTypeRef) -> CFDictionaryRef;
type ChannelGetStringFn = unsafe extern "C" fn(CFDictionaryRef) -> CFStringRef;
type SimpleGetIntegerFn = unsafe extern "C" fn(CFDictionaryRef, i32) -> i64;

const LIB_PATHS: [&str; 2] = [
    "/usr/lib/libIOReport.dylib",
    "/System/Library/PrivateFrameworks/IOReport.framework/IOReport",
];

const GROUP: &str = "Energy Model";

pub struct IoReportClient {
    _lib: Library,
    create_samples: CreateSamplesFn,
    create_samples_delta: CreateSamplesDeltaFn,
    channel_get_group: ChannelGetStringFn,
    channel_get_name: ChannelGetStringFn,
    channel_get_unit: ChannelGetStringFn,
    simple_get_integer: SimpleGetIntegerFn,

    channels: CFMutableDictionaryRef,
    /// Ownership of this out-parameter is undocumented; leak this single one-time
    /// dictionary rather than risk a double release.
    _sub_channels: CFMutableDictionaryRef,
    subscription: *mut c_void,
    prev_sample: Option<CFDictionaryRef>,
    prev_instant: Option<Instant>,
}

impl IoReportClient {
    pub fn open() -> Result<Self, String> {
        let mut last_err = String::from("libIOReport.dylib not found");
        for path in LIB_PATHS {
            match unsafe { Library::new(path) } {
                Ok(lib) => match Self::from_library(lib) {
                    Ok(client) => return Ok(client),
                    Err(err) => last_err = err,
                },
                Err(err) => last_err = format!("{path}: {err}"),
            }
        }
        Err(last_err)
    }

    fn from_library(lib: Library) -> Result<Self, String> {
        // Keep the borrowed Symbols inside this block so `lib` can be moved into
        // the struct afterwards. Function pointers remain valid while _lib lives.
        let (fns, channels, sub_channels, subscription) = {
            unsafe {
                let copy_channels: Symbol<CopyChannelsFn> = lib
                    .get(b"IOReportCopyChannelsInGroup\0")
                    .map_err(|e| format!("IOReportCopyChannelsInGroup: {e}"))?;
                let create_subscription: Symbol<CreateSubscriptionFn> = lib
                    .get(b"IOReportCreateSubscription\0")
                    .map_err(|e| format!("IOReportCreateSubscription: {e}"))?;
                let create_samples: Symbol<CreateSamplesFn> =
                    lib.get(b"IOReportCreateSamples\0")
                        .map_err(|e| format!("IOReportCreateSamples: {e}"))?;
                let create_samples_delta: Symbol<CreateSamplesDeltaFn> = lib
                    .get(b"IOReportCreateSamplesDelta\0")
                    .map_err(|e| format!("IOReportCreateSamplesDelta: {e}"))?;
                let channel_get_group: Symbol<ChannelGetStringFn> = lib
                    .get(b"IOReportChannelGetGroup\0")
                    .map_err(|e| format!("IOReportChannelGetGroup: {e}"))?;
                let channel_get_name: Symbol<ChannelGetStringFn> = lib
                    .get(b"IOReportChannelGetChannelName\0")
                    .map_err(|e| format!("IOReportChannelGetChannelName: {e}"))?;
                let channel_get_unit: Symbol<ChannelGetStringFn> = lib
                    .get(b"IOReportChannelGetUnitLabel\0")
                    .map_err(|e| format!("IOReportChannelGetUnitLabel: {e}"))?;
                let simple_get_integer: Symbol<SimpleGetIntegerFn> = lib
                    .get(b"IOReportSimpleGetIntegerValue\0")
                    .map_err(|e| format!("IOReportSimpleGetIntegerValue: {e}"))?;

                let group =
                    cf::cfstring(GROUP).ok_or_else(|| "CFString allocation failed".to_string())?;
                let channels = (*copy_channels)(group, std::ptr::null(), 0, 0, 0);
                CFRelease(group as CFTypeRef);
                if channels.is_null() {
                    return Err(format!("IOReport group '{GROUP}' returned no channels"));
                }

                let mut sub_channels: CFMutableDictionaryRef = std::ptr::null_mut();
                let mut tag: u64 = 0;
                let subscription = (*create_subscription)(
                    std::ptr::null_mut(),
                    channels,
                    &mut sub_channels,
                    &mut tag,
                    std::ptr::null(),
                );
                if subscription.is_null() {
                    CFRelease(channels as CFTypeRef);
                    return Err("IOReportCreateSubscription failed".to_string());
                }

                (
                    (
                        *create_samples,
                        *create_samples_delta,
                        *channel_get_group,
                        *channel_get_name,
                        *channel_get_unit,
                        *simple_get_integer,
                    ),
                    channels,
                    sub_channels,
                    subscription,
                )
            }
        };

        let (
            create_samples,
            create_samples_delta,
            channel_get_group,
            channel_get_name,
            channel_get_unit,
            simple_get_integer,
        ) = fns;

        Ok(Self {
            _lib: lib,
            create_samples,
            create_samples_delta,
            channel_get_group,
            channel_get_name,
            channel_get_unit,
            simple_get_integer,
            channels,
            _sub_channels: sub_channels,
            subscription,
            prev_sample: None,
            prev_instant: None,
        })
    }

    pub fn sample(&mut self) -> Vec<RailValue> {
        let now = Instant::now();
        let channels = if self._sub_channels.is_null() {
            self.channels
        } else {
            self._sub_channels
        };
        let current =
            unsafe { (self.create_samples)(self.subscription, channels, std::ptr::null()) };
        if current.is_null() {
            return Vec::new();
        }

        let prev = self.prev_sample.take();
        let prev_instant = self.prev_instant.take();
        self.prev_sample = Some(current);
        self.prev_instant = Some(now);

        let (Some(prev), Some(prev_instant)) = (prev, prev_instant) else {
            return Vec::new();
        };
        let dt_ms = now.duration_since(prev_instant).as_secs_f64() * 1000.0;
        if dt_ms < 1.0 {
            unsafe { CFRelease(prev as CFTypeRef) };
            return Vec::new();
        }

        let delta = unsafe { (self.create_samples_delta)(prev, current, std::ptr::null()) };
        unsafe { CFRelease(prev as CFTypeRef) };
        if delta.is_null() {
            return Vec::new();
        }
        let _guard = CFGuard(delta as CFTypeRef);
        self.parse_delta(delta, dt_ms)
    }

    fn parse_delta(&self, delta: CFDictionaryRef, dt_ms: f64) -> Vec<RailValue> {
        let Some(array) = cf::dict_get_array(delta, "IOReportChannels") else {
            return Vec::new();
        };
        let n = unsafe { CFArrayGetCount(array) }.max(0) as usize;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let channel = unsafe { CFArrayGetValueAtIndex(array, i as isize) } as CFDictionaryRef;
            if channel.is_null() {
                continue;
            }
            let group = unsafe { (self.channel_get_group)(channel) };
            let group = cf::cfstring_to_string(group).unwrap_or_default();
            if group != GROUP {
                continue;
            }
            let name = unsafe { (self.channel_get_name)(channel) };
            let Some(name) = cf::cfstring_to_string(name) else {
                continue;
            };
            let unit = unsafe { (self.channel_get_unit)(channel) };
            let unit = cf::cfstring_to_string(unit).unwrap_or_default();
            let value = unsafe { (self.simple_get_integer)(channel, 0) };
            if value == i64::MIN || value < 0 {
                continue;
            }
            let Some(factor_mj) = unit_to_mj_factor(&unit) else {
                continue;
            };
            let mj = value as f64 * factor_mj;
            let w = (mj / dt_ms) as f32;
            if !w.is_finite() || w < 0.0 {
                continue;
            }
            out.push(RailValue {
                name,
                group: group.clone(),
                w,
                unit,
                source: RailSource::IoReport,
            });
        }
        out
    }

    /// Take a short two-sample probe for `--list-sensors`.
    pub fn probe(&mut self) -> Vec<RailValue> {
        self.reset();
        let _ = self.sample();
        std::thread::sleep(Duration::from_millis(300));
        self.sample()
    }

    pub fn reset(&mut self) {
        if let Some(prev) = self.prev_sample.take() {
            unsafe { CFRelease(prev as CFTypeRef) };
        }
        self.prev_instant = None;
    }
}

impl Drop for IoReportClient {
    fn drop(&mut self) {
        self.reset();
        unsafe {
            if !self.subscription.is_null() {
                CFRelease(self.subscription as CFTypeRef);
            }
            if !self.channels.is_null() {
                CFRelease(self.channels as CFTypeRef);
            }
        }
    }
}

fn unit_to_mj_factor(unit: &str) -> Option<f64> {
    match unit.trim() {
        "mJ" => Some(1.0),
        "uJ" | "µJ" => Some(0.001),
        "nJ" => Some(1e-6),
        "J" => Some(1000.0),
        _ => None,
    }
}

#[derive(Clone, Debug, Default)]
pub struct ComponentPower {
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
}

impl ComponentPower {
    pub fn from_rails(rails: &[RailValue]) -> Self {
        let find = |name: &str| rails.iter().find(|r| r.name == name).map(|r| r.w);

        let cores = per_core_values(rails);
        let mut ecpu = find("ECPU");
        let mut pcpu = find("PCPU");
        if ecpu.is_none() && !cores.0.is_empty() {
            ecpu = Some(cores.0.iter().sum());
        }
        if pcpu.is_none() && !cores.1.is_empty() {
            pcpu = Some(cores.1.iter().sum());
        }

        let cpu = find("CPU Energy").or(match (ecpu, pcpu) {
            (Some(e), Some(p)) => Some(e + p),
            (Some(e), None) => Some(e),
            (None, Some(p)) => Some(p),
            (None, None) => None,
        });

        let gpu = match (find("GPU"), find("GPU Energy")) {
            (Some(v), Some(fine)) if v == 0.0 && fine > 0.0 => Some(fine),
            (Some(v), _) => Some(v),
            (None, Some(fine)) => Some(fine),
            (None, None) => None,
        };

        let display = sum_options([find("DISP"), find("DISPEXT")]);
        let other = sum_options([
            find("ISP"),
            find("AVE"),
            find("MSR"),
            find("VDEC"),
            find("SOC_AON"),
            find("SOC_REST"),
            sum_options([
                find("PCIe Port 0 Energy"),
                find("PCIe Port 1 Energy"),
                find("apciec0 Energy"),
                find("apciec1 Energy"),
            ]),
        ]);

        Self {
            cpu_w: cpu,
            ecpu_w: ecpu,
            pcpu_w: pcpu,
            core_e_w: cores.0,
            core_p_w: cores.1,
            gpu_w: gpu,
            ane_w: find("ANE"),
            dram_w: find("DRAM"),
            display_w: display,
            other_w: other,
        }
    }
}

fn sum_options<const N: usize>(values: [Option<f32>; N]) -> Option<f32> {
    let mut sum = 0.0;
    let mut any = false;
    for v in values.into_iter().flatten() {
        sum += v;
        any = true;
    }
    if any {
        Some(sum)
    } else {
        None
    }
}

fn per_core_values(rails: &[RailValue]) -> (Vec<f32>, Vec<f32>) {
    let mut e: Vec<(usize, f32)> = Vec::new();
    let mut p: Vec<(usize, f32)> = Vec::new();
    for rail in rails {
        if let Some((kind, idx)) = parse_core_name(&rail.name) {
            match kind {
                'E' => e.push((idx, rail.w)),
                'P' => p.push((idx, rail.w)),
                _ => {}
            }
        }
    }
    (fill_cores(e), fill_cores(p))
}

fn fill_cores(mut values: Vec<(usize, f32)>) -> Vec<f32> {
    if values.is_empty() {
        return Vec::new();
    }
    values.sort_by_key(|(idx, _)| *idx);
    let max = values.iter().map(|(idx, _)| *idx).max().unwrap_or(0);
    let mut out = vec![0.0; max + 1];
    for (idx, w) in values {
        out[idx] = w;
    }
    out
}

fn parse_core_name(name: &str) -> Option<(char, usize)> {
    let bytes = name.as_bytes();
    if bytes.len() < 5 {
        return None;
    }
    if bytes[0] != b'E' && bytes[0] != b'P' {
        return None;
    }
    if &bytes[1..4] != b"CPU" {
        return None;
    }
    let digits = &bytes[4..];
    if digits.is_empty() || !digits.iter().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let idx = std::str::from_utf8(digits).ok()?.parse().ok()?;
    Some((bytes[0] as char, idx))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rail(name: &str, w: f32) -> RailValue {
        RailValue {
            name: name.to_string(),
            group: GROUP.to_string(),
            w,
            unit: "mJ".to_string(),
            source: RailSource::IoReport,
        }
    }

    #[test]
    fn unit_conversion() {
        assert_eq!(unit_to_mj_factor("mJ"), Some(1.0));
        assert_eq!(unit_to_mj_factor("uJ"), Some(0.001));
        assert_eq!(unit_to_mj_factor("nJ"), Some(1e-6));
        assert_eq!(unit_to_mj_factor("bogus"), None);
    }

    #[test]
    fn parses_only_per_core_channels() {
        assert_eq!(parse_core_name("ECPU0"), Some(('E', 0)));
        assert_eq!(parse_core_name("PCPU12"), Some(('P', 12)));
        assert_eq!(parse_core_name("ECPU"), None);
        assert_eq!(parse_core_name("ECPU0_SRAM"), None);
        assert_eq!(parse_core_name("ECPM"), None);
        assert_eq!(parse_core_name("ECPUDTL00"), None);
    }

    #[test]
    fn classifies_clusters_and_per_core() {
        let rails = vec![
            rail("CPU Energy", 0.131),
            rail("ECPU", 0.085),
            rail("PCPU", 0.047),
            rail("ECPU0", 0.015),
            rail("ECPU1", 0.016),
            rail("PCPU0", 0.011),
            rail("GPU", 0.0),
            rail("GPU Energy", 0.0005),
            rail("ANE", 0.0),
            rail("DRAM", 0.059),
            rail("DISP", 0.016),
            rail("DISPEXT", 0.001),
            rail("ISP", 0.008),
            rail("AMCC", 0.054),
        ];
        let c = ComponentPower::from_rails(&rails);
        assert!((c.cpu_w.unwrap() - 0.131).abs() < 1e-6);
        assert!((c.ecpu_w.unwrap() - 0.085).abs() < 1e-6);
        assert!((c.pcpu_w.unwrap() - 0.047).abs() < 1e-6);
        assert_eq!(c.core_e_w, vec![0.015, 0.016]);
        assert_eq!(c.core_p_w, vec![0.011]);
        assert!((c.gpu_w.unwrap() - 0.0005).abs() < 1e-6);
        assert!((c.display_w.unwrap() - 0.017).abs() < 1e-6);
        assert!((c.other_w.unwrap() - 0.008).abs() < 1e-6); // AMCC intentionally excluded
    }

    #[test]
    fn falls_back_to_cluster_sums() {
        let rails = vec![
            rail("ECPU0", 0.015),
            rail("ECPU1", 0.016),
            rail("PCPU0", 0.011),
        ];
        let c = ComponentPower::from_rails(&rails);
        assert!((c.ecpu_w.unwrap() - 0.031).abs() < 1e-6);
        assert!((c.pcpu_w.unwrap() - 0.011).abs() < 1e-6);
        assert!((c.cpu_w.unwrap() - 0.042).abs() < 1e-6);
    }
}
