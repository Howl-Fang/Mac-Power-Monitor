//! AppleSMC user-client access.
//!
//! The SMC protocol is private but stable across Intel and Apple Silicon. Keys
//! are four ASCII bytes packed big-endian; data types are also four ASCII bytes.

use std::collections::HashMap;

use crate::collectors::ffi::iokit::{self, IoConnect, KERN_SUCCESS};
use crate::model::SmcValues;

const KERNEL_INDEX_SMC: u32 = 2;
const SMC_CMD_READ_BYTES: u8 = 5;
const SMC_CMD_READ_KEYINFO: u8 = 9;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct SmcVers {
    major: u8,
    minor: u8,
    build: u8,
    reserved: u8,
    release: u16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct SmcPLimit {
    version: u16,
    length: u16,
    cpu_p_limit: u32,
    gpu_p_limit: u32,
    mem_p_limit: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct SmcKeyInfo {
    data_size: u32,
    data_type: u32,
    data_attributes: u8,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
struct SmcKeyData {
    key: u32,
    vers: SmcVers,
    p_limit: SmcPLimit,
    key_info: SmcKeyInfo,
    result: u8,
    status: u8,
    data8: u8,
    data32: u32,
    bytes: [u8; 32],
}

#[derive(Clone, Copy, Debug)]
struct KeyMeta {
    data_size: u32,
    data_type: u32,
}

pub struct SmcClient {
    conn: IoConnect,
    cache: HashMap<u32, KeyMeta>,
}

impl SmcClient {
    pub fn open() -> Result<Self, String> {
        if std::mem::size_of::<SmcKeyData>() != 80 {
            return Err(format!(
                "internal SMC struct size is {}, expected 80",
                std::mem::size_of::<SmcKeyData>()
            ));
        }
        let service = iokit::open_service("AppleSMC")
            .ok_or_else(|| "AppleSMC service not found".to_string())?;
        let mut conn: IoConnect = 0;
        let kr = unsafe { iokit::IOServiceOpen(service, iokit::mach_task_self(), 0, &mut conn) };
        unsafe { iokit::IOObjectRelease(service) };
        if kr != KERN_SUCCESS || conn == 0 {
            return Err(format!("IOServiceOpen(AppleSMC) failed: 0x{kr:x}"));
        }
        Ok(Self {
            conn,
            cache: HashMap::new(),
        })
    }

    fn call(&self, input: &mut SmcKeyData) -> Option<SmcKeyData> {
        let mut output = SmcKeyData::default();
        let mut output_size = std::mem::size_of::<SmcKeyData>();
        let kr = unsafe {
            iokit::IOConnectCallStructMethod(
                self.conn,
                KERNEL_INDEX_SMC,
                input as *const SmcKeyData as *const std::os::raw::c_void,
                std::mem::size_of::<SmcKeyData>(),
                &mut output as *mut SmcKeyData as *mut std::os::raw::c_void,
                &mut output_size,
            )
        };
        if kr != KERN_SUCCESS {
            return None;
        }
        Some(output)
    }

    fn read_key_info(&mut self, key: u32) -> Option<KeyMeta> {
        if let Some(meta) = self.cache.get(&key).copied() {
            return Some(meta);
        }
        let mut input = SmcKeyData {
            key,
            data8: SMC_CMD_READ_KEYINFO,
            ..Default::default()
        };
        let output = self.call(&mut input)?;
        if output.result != 0 || output.key_info.data_size == 0 || output.key_info.data_size > 32 {
            return None;
        }
        let meta = KeyMeta {
            data_size: output.key_info.data_size,
            data_type: output.key_info.data_type,
        };
        self.cache.insert(key, meta);
        Some(meta)
    }

    pub fn read_bytes(&mut self, key: &[u8; 4]) -> Option<(u32, Vec<u8>)> {
        let key_u32 = fourcc(key);
        for attempt in 0..2 {
            let meta = self.read_key_info(key_u32)?;
            let mut input = SmcKeyData {
                key: key_u32,
                data8: SMC_CMD_READ_BYTES,
                ..Default::default()
            };
            input.key_info.data_size = meta.data_size;
            let output = self.call(&mut input)?;
            if output.result == 0 {
                let n = meta.data_size.min(32) as usize;
                return Some((meta.data_type, output.bytes[..n].to_vec()));
            }
            // Stale key info: drop the cache and retry once.
            self.cache.remove(&key_u32);
            if attempt == 1 {
                break;
            }
        }
        None
    }

    pub fn read_f32(&mut self, key: &[u8; 4], plausible: impl Fn(f32) -> bool) -> Option<f32> {
        let (data_type, bytes) = self.read_bytes(key)?;
        decode_f32(data_type, &bytes, plausible)
    }

    pub fn read_power_w(&mut self, key: &[u8; 4]) -> Option<f32> {
        // Power rails are zero or a plausible positive watt value. Rejecting tiny
        // non-zero values is what disambiguates little-endian vs big-endian `flt`.
        self.read_f32(key, plausible_power)
    }

    pub fn read_power(&mut self) -> SmcValues {
        SmcValues {
            pstr_w: self.read_power_w(b"PSTR"),
            ppbr_w: self.read_power_w(b"PPBR"),
            pdtr_w: self.read_power_w(b"PDTR"),
        }
    }

    pub fn probe(&mut self) -> Vec<SmcProbe> {
        const KEYS: [&[u8; 4]; 8] = [
            b"PSTR", b"PPBR", b"PDTR", b"PCPT", b"PC0R", b"PG0R", b"PBTW", b"PBLM",
        ];
        let mut out = Vec::new();
        for key in KEYS {
            let result = self.read_bytes(key);
            let (available, type_name, value) = match result {
                Some((data_type, bytes)) => {
                    let value = decode_f32(data_type, &bytes, plausible_power);
                    (true, type_name(data_type), value)
                }
                None => (false, String::new(), None),
            };
            out.push(SmcProbe {
                key: String::from_utf8_lossy(key).to_string(),
                available,
                type_name,
                value,
            });
        }
        out
    }
}

impl Drop for SmcClient {
    fn drop(&mut self) {
        unsafe {
            iokit::IOServiceClose(self.conn);
        }
    }
}

#[derive(Clone, Debug)]
pub struct SmcProbe {
    pub key: String,
    pub available: bool,
    pub type_name: String,
    pub value: Option<f32>,
}

pub fn fourcc(key: &[u8; 4]) -> u32 {
    ((key[0] as u32) << 24) | ((key[1] as u32) << 16) | ((key[2] as u32) << 8) | key[3] as u32
}

pub fn type_name(data_type: u32) -> String {
    let bytes = [
        (data_type >> 24) as u8,
        (data_type >> 16) as u8,
        (data_type >> 8) as u8,
        data_type as u8,
    ];
    String::from_utf8_lossy(&bytes).trim_end().to_string()
}

fn decode_f32(data_type: u32, bytes: &[u8], plausible: impl Fn(f32) -> bool) -> Option<f32> {
    let name = type_name(data_type);
    match name.as_str() {
        "flt" => decode_flt(bytes, plausible),
        "sp78" => {
            if bytes.len() < 2 {
                return None;
            }
            let raw = i16::from_be_bytes([bytes[0], bytes[1]]);
            natural(raw as f32 / 256.0, plausible)
        }
        "fpe2" => {
            if bytes.len() < 2 {
                return None;
            }
            let raw = u16::from_be_bytes([bytes[0], bytes[1]]);
            natural(raw as f32 / 4.0, plausible)
        }
        "ui8" => bytes.first().map(|&b| b as f32).filter(|v| plausible(*v)),
        "ui16" => {
            if bytes.len() < 2 {
                return None;
            }
            let raw = u16::from_be_bytes([bytes[0], bytes[1]]);
            natural(raw as f32, plausible)
        }
        "ui32" => {
            if bytes.len() < 4 {
                return None;
            }
            let raw = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            natural(raw as f32, plausible)
        }
        _ => None,
    }
}

fn decode_flt(bytes: &[u8], plausible: impl Fn(f32) -> bool) -> Option<f32> {
    if bytes.len() < 4 {
        return None;
    }
    let b: [u8; 4] = bytes[..4].try_into().ok()?;
    let le = f32::from_le_bytes(b);
    let be = f32::from_be_bytes(b);
    let le_ok = le.is_finite() && plausible(le);
    let be_ok = be.is_finite() && plausible(be);
    match (le_ok, be_ok) {
        (true, false) => Some(le),
        (false, true) => Some(be),
        (true, true) => {
            if cfg!(target_arch = "aarch64") {
                Some(le)
            } else {
                Some(be)
            }
        }
        (false, false) => None,
    }
}

fn natural(value: f32, plausible: impl Fn(f32) -> bool) -> Option<f32> {
    if value.is_finite() && plausible(value) {
        Some(value)
    } else {
        None
    }
}

fn plausible_power(v: f32) -> bool {
    v.is_finite() && (v == 0.0 || (0.01..=1000.0).contains(&v))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_layout_matches_c() {
        assert_eq!(std::mem::size_of::<SmcKeyData>(), 80);
        assert_eq!(std::mem::offset_of!(SmcKeyData, key), 0);
        assert_eq!(std::mem::offset_of!(SmcKeyData, key_info), 28);
        assert_eq!(std::mem::offset_of!(SmcKeyData, result), 40);
        assert_eq!(std::mem::offset_of!(SmcKeyData, data32), 44);
        assert_eq!(std::mem::offset_of!(SmcKeyData, bytes), 48);
    }

    #[test]
    fn fourcc_is_big_endian_ascii() {
        assert_eq!(fourcc(b"PSTR"), 0x5053_5452);
        assert_eq!(type_name(0x666c_7420), "flt");
    }

    #[test]
    fn decodes_little_endian_float_from_m3() {
        // 3.851 W as observed in the M3 PSTR probe.
        let v = decode_flt(&[0x35, 0x7a, 0x76, 0x40], plausible_power).unwrap();
        assert!((v - 3.851).abs() < 0.001, "got {v}");
    }

    #[test]
    fn decodes_big_endian_float_for_intel() {
        let be = 3.851f32.to_be_bytes();
        let v = decode_flt(&be, plausible_power).unwrap();
        assert!((v - 3.851).abs() < 0.001, "got {v}");
    }

    #[test]
    fn decodes_fixed_point_types() {
        assert_eq!(decode_f32(0x7370_3738, &[0x0c, 0x80], |_| true), Some(12.5)); // sp78
        assert_eq!(decode_f32(0x6670_6532, &[0x00, 0x10], |_| true), Some(4.0));
        // fpe2
    }
}
