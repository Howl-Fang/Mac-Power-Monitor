//! Raw IOKit declarations used by the collectors.

use std::os::raw::{c_char, c_void};

use core_foundation_sys::base::{CFAllocatorRef, CFTypeRef};
use core_foundation_sys::dictionary::{CFDictionaryRef, CFMutableDictionaryRef};

pub type IoObject = u32;
pub type IoService = u32;
pub type IoConnect = u32;
pub type KernReturn = i32;
pub type MachPort = u32;

pub const KERN_SUCCESS: KernReturn = 0;
pub const IO_OBJECT_NULL: IoObject = 0;
pub const K_IO_MAIN_PORT_DEFAULT: MachPort = 0;

#[link(name = "IOKit", kind = "framework")]
extern "C" {
    pub static mach_task_self_: MachPort;

    pub fn IOServiceMatching(name: *const c_char) -> CFMutableDictionaryRef;
    pub fn IOServiceGetMatchingService(main_port: MachPort, matching: CFDictionaryRef)
        -> IoService;
    pub fn IOServiceOpen(
        service: IoService,
        owning_task: MachPort,
        type_: u32,
        connect: *mut IoConnect,
    ) -> KernReturn;
    pub fn IOServiceClose(connect: IoConnect) -> KernReturn;
    pub fn IOObjectRelease(object: IoObject) -> KernReturn;

    pub fn IORegistryEntryCreateCFProperties(
        entry: IoObject,
        properties: *mut CFMutableDictionaryRef,
        allocator: CFAllocatorRef,
        options: u32,
    ) -> KernReturn;

    pub fn IOConnectCallStructMethod(
        connection: IoConnect,
        selector: u32,
        input_struct: *const c_void,
        input_struct_cnt: usize,
        output_struct: *mut c_void,
        output_struct_cnt: *mut usize,
    ) -> KernReturn;

    pub fn IOPSCopyPowerSourcesInfo() -> CFTypeRef;
    pub fn IOPSCopyPowerSourcesList(blob: CFTypeRef) -> core_foundation_sys::array::CFArrayRef;
    pub fn IOPSGetPowerSourceDescription(
        blob: CFTypeRef,
        power_source: CFTypeRef,
    ) -> CFDictionaryRef;
}

pub fn mach_task_self() -> MachPort {
    unsafe { mach_task_self_ }
}

pub fn open_service(name: &str) -> Option<IoService> {
    let cname = std::ffi::CString::new(name).ok()?;
    let service = unsafe {
        IOServiceGetMatchingService(K_IO_MAIN_PORT_DEFAULT, IOServiceMatching(cname.as_ptr()))
    };
    if service == IO_OBJECT_NULL {
        None
    } else {
        Some(service)
    }
}
