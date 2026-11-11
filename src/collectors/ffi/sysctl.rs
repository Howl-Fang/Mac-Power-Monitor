//! Minimal sysctl/gethostname helpers.

use std::ffi::CString;
use std::os::raw::{c_char, c_void};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

extern "C" {
    fn sysctlbyname(
        name: *const c_char,
        oldp: *mut c_void,
        oldlenp: *mut usize,
        newp: *mut c_void,
        newlen: usize,
    ) -> i32;
    fn gethostname(name: *mut c_char, namelen: usize) -> i32;
}

fn cname(name: &str) -> Option<CString> {
    CString::new(name).ok()
}

pub fn sysctl_bytes(name: &str) -> Option<Vec<u8>> {
    let name = cname(name)?;
    let mut len: usize = 0;
    let rc = unsafe {
        sysctlbyname(
            name.as_ptr(),
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || len == 0 || len > 1 << 20 {
        return None;
    }
    let mut buf = vec![0u8; len];
    let rc = unsafe {
        sysctlbyname(
            name.as_ptr(),
            buf.as_mut_ptr() as *mut c_void,
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        None
    } else {
        buf.truncate(len);
        Some(buf)
    }
}

pub fn sysctl_string(name: &str) -> Option<String> {
    let bytes = sysctl_bytes(name)?;
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    Some(String::from_utf8_lossy(&bytes[..end]).trim().to_string())
}

pub fn sysctl_i32(name: &str) -> Option<i32> {
    let bytes = sysctl_bytes(name)?;
    if bytes.len() < 4 {
        return None;
    }
    Some(i32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

pub fn sysctl_u64(name: &str) -> Option<u64> {
    let bytes = sysctl_bytes(name)?;
    if bytes.len() < 8 {
        return None;
    }
    let mut b = [0u8; 8];
    b.copy_from_slice(&bytes[..8]);
    Some(u64::from_ne_bytes(b))
}

pub fn sysctl_bool(name: &str) -> Option<bool> {
    sysctl_i32(name).map(|v| v != 0)
}

pub fn hostname() -> Option<String> {
    let mut buf = vec![0 as c_char; 256];
    let rc = unsafe { gethostname(buf.as_mut_ptr(), buf.len()) };
    if rc != 0 {
        return None;
    }
    let bytes: Vec<u8> = buf
        .into_iter()
        .map(|c| c as u8)
        .take_while(|&b| b != 0)
        .collect();
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

pub fn boot_time() -> Option<SystemTime> {
    let bytes = sysctl_bytes("kern.boottime")?;
    if bytes.len() < 16 {
        return None;
    }
    let secs = i64::from_ne_bytes(bytes[0..8].try_into().ok()?);
    if secs <= 0 {
        return None;
    }
    UNIX_EPOCH.checked_add(Duration::from_secs(secs as u64))
}

pub fn uptime_secs() -> Option<u64> {
    let boot = boot_time()?;
    SystemTime::now()
        .duration_since(boot)
        .ok()
        .map(|d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_system_values() {
        assert!(sysctl_string("kern.osproductversion").is_some());
        assert!(sysctl_u64("hw.memsize").unwrap_or(0) > 0);
        assert!(sysctl_i32("hw.ncpu").unwrap_or(0) > 0);
    }
}
