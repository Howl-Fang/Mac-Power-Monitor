//! Thin, explicit CoreFoundation helpers.
//!
//! All functions here take borrowed CoreFoundation references unless their name
//! says otherwise. Callers must not release borrowed values. Objects returned
//! by functions ending in `_owned` are +1 and should be wrapped in [`CFGuard`].

use std::ffi::CString;
use std::os::raw::{c_char, c_void};

use core_foundation_sys::array::{
    CFArrayGetCount, CFArrayGetTypeID, CFArrayGetValueAtIndex, CFArrayRef,
};
use core_foundation_sys::base::{kCFAllocatorDefault, CFGetTypeID, CFRelease, CFTypeID, CFTypeRef};
use core_foundation_sys::dictionary::{CFDictionaryGetValue, CFDictionaryRef};
use core_foundation_sys::number::{
    kCFNumberFloat64Type, kCFNumberSInt64Type, CFBooleanGetTypeID, CFBooleanGetValue, CFBooleanRef,
    CFNumberGetTypeID, CFNumberGetValue, CFNumberRef,
};
use core_foundation_sys::string::{
    kCFStringEncodingUTF8, CFStringCreateWithCString, CFStringGetCString, CFStringGetLength,
    CFStringGetMaximumSizeForEncoding, CFStringGetTypeID, CFStringRef,
};

/// Owns a +1 CoreFoundation object and releases it on drop.
pub struct CFGuard(pub CFTypeRef);

impl Drop for CFGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { CFRelease(self.0) };
        }
    }
}

/// Create a +1 CFString. The caller owns the returned reference.
pub fn cfstring(s: &str) -> Option<CFStringRef> {
    let c = CString::new(s).ok()?;
    let r = unsafe {
        CFStringCreateWithCString(kCFAllocatorDefault, c.as_ptr(), kCFStringEncodingUTF8)
    };
    if r.is_null() {
        None
    } else {
        Some(r)
    }
}

/// Copy a borrowed CFString into a Rust String.
pub fn cfstring_to_string(s: CFStringRef) -> Option<String> {
    if s.is_null() {
        return None;
    }
    let len = unsafe { CFStringGetLength(s) };
    let max = unsafe { CFStringGetMaximumSizeForEncoding(len, kCFStringEncodingUTF8) } + 1;
    if max <= 0 {
        return None;
    }
    let mut buf = vec![0 as c_char; max as usize];
    let ok = unsafe { CFStringGetCString(s, buf.as_mut_ptr(), max, kCFStringEncodingUTF8) };
    if ok == 0 {
        return None;
    }
    let bytes: Vec<u8> = buf
        .into_iter()
        .map(|c| c as u8)
        .take_while(|&b| b != 0)
        .collect();
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Look up a borrowed value in a CFDictionary.
pub fn dict_get(dict: CFDictionaryRef, key: &str) -> Option<CFTypeRef> {
    if dict.is_null() {
        return None;
    }
    let key = cfstring(key)?;
    let value = unsafe { CFDictionaryGetValue(dict, key as *const c_void) };
    unsafe { CFRelease(key as CFTypeRef) };
    if value.is_null() {
        None
    } else {
        Some(value)
    }
}

pub fn type_id(value: CFTypeRef) -> CFTypeID {
    unsafe { CFGetTypeID(value) }
}

pub fn is_dictionary(value: CFTypeRef) -> bool {
    type_id(value) == unsafe { core_foundation_sys::dictionary::CFDictionaryGetTypeID() }
}

pub fn is_array(value: CFTypeRef) -> bool {
    type_id(value) == unsafe { CFArrayGetTypeID() }
}

pub fn is_number(value: CFTypeRef) -> bool {
    type_id(value) == unsafe { CFNumberGetTypeID() }
}

pub fn is_boolean(value: CFTypeRef) -> bool {
    type_id(value) == unsafe { CFBooleanGetTypeID() }
}

pub fn is_string(value: CFTypeRef) -> bool {
    type_id(value) == unsafe { CFStringGetTypeID() }
}

pub fn dict_get_i64(dict: CFDictionaryRef, key: &str) -> Option<i64> {
    let value = dict_get(dict, key)?;
    if !is_number(value) {
        return None;
    }
    let mut out: i64 = 0;
    let ok = unsafe {
        CFNumberGetValue(
            value as CFNumberRef,
            kCFNumberSInt64Type,
            &mut out as *mut i64 as *mut c_void,
        )
    };
    if ok {
        Some(out)
    } else {
        let mut f: f64 = 0.0;
        let ok = unsafe {
            CFNumberGetValue(
                value as CFNumberRef,
                kCFNumberFloat64Type,
                &mut f as *mut f64 as *mut c_void,
            )
        };
        if ok && f.is_finite() {
            Some(f as i64)
        } else {
            None
        }
    }
}

pub fn dict_get_f64(dict: CFDictionaryRef, key: &str) -> Option<f64> {
    let value = dict_get(dict, key)?;
    if !is_number(value) {
        return None;
    }
    let mut out: f64 = 0.0;
    let ok = unsafe {
        CFNumberGetValue(
            value as CFNumberRef,
            kCFNumberFloat64Type,
            &mut out as *mut f64 as *mut c_void,
        )
    };
    if ok && out.is_finite() {
        Some(out)
    } else {
        None
    }
}

pub fn dict_get_bool(dict: CFDictionaryRef, key: &str) -> Option<bool> {
    let value = dict_get(dict, key)?;
    if !is_boolean(value) {
        return None;
    }
    Some(unsafe { CFBooleanGetValue(value as CFBooleanRef) })
}

pub fn dict_get_string(dict: CFDictionaryRef, key: &str) -> Option<String> {
    let value = dict_get(dict, key)?;
    if !is_string(value) {
        return None;
    }
    cfstring_to_string(value as CFStringRef)
}

pub fn dict_get_dict(dict: CFDictionaryRef, key: &str) -> Option<CFDictionaryRef> {
    let value = dict_get(dict, key)?;
    if is_dictionary(value) {
        Some(value as CFDictionaryRef)
    } else {
        None
    }
}

pub fn dict_get_array(dict: CFDictionaryRef, key: &str) -> Option<CFArrayRef> {
    let value = dict_get(dict, key)?;
    if is_array(value) {
        Some(value as CFArrayRef)
    } else {
        None
    }
}

pub fn array_len(array: CFArrayRef) -> usize {
    if array.is_null() {
        0
    } else {
        unsafe { CFArrayGetCount(array) }.max(0) as usize
    }
}

pub fn array_get(array: CFArrayRef, index: usize) -> Option<CFTypeRef> {
    if array.is_null() || index >= array_len(array) {
        return None;
    }
    let value = unsafe { CFArrayGetValueAtIndex(array, index as isize) };
    if value.is_null() {
        None
    } else {
        Some(value)
    }
}

pub fn array_get_i64(array: CFArrayRef, index: usize) -> Option<i64> {
    let value = array_get(array, index)?;
    if !is_number(value) {
        return None;
    }
    let mut out: i64 = 0;
    let ok = unsafe {
        CFNumberGetValue(
            value as CFNumberRef,
            kCFNumberSInt64Type,
            &mut out as *mut i64 as *mut c_void,
        )
    };
    if ok {
        Some(out)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core_foundation_sys::base::CFRetain;

    #[test]
    fn cfstring_round_trip() {
        let s = cfstring("MacPower \u{1f5a5}").unwrap();
        let back = cfstring_to_string(s).unwrap();
        unsafe { CFRelease(s as CFTypeRef) };
        assert_eq!(back, "MacPower \u{1f5a5}");
    }

    #[test]
    fn null_dictionary_is_safe() {
        assert!(dict_get(std::ptr::null(), "Voltage").is_none());
        assert_eq!(array_len(std::ptr::null()), 0);
    }

    #[test]
    fn guard_releases_retained_object() {
        let s = cfstring("guard").unwrap();
        let retained = unsafe { CFRetain(s as CFTypeRef) };
        let _guard = CFGuard(retained);
        unsafe { CFRelease(s as CFTypeRef) };
        // If the guard double-released, the test process would crash.
    }
}
