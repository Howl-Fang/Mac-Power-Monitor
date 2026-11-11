//! Static system information via sysctl.

use crate::collectors::ffi::sysctl;
use crate::model::SystemInfo;

impl SystemInfo {
    pub fn detect() -> Self {
        Self {
            hostname: sysctl::hostname().unwrap_or_else(|| "unknown".to_string()),
            chip: sysctl::sysctl_string("machdep.cpu.brand_string")
                .unwrap_or_else(|| "Unknown SoC".to_string()),
            model: sysctl::sysctl_string("hw.model").unwrap_or_default(),
            os_version: sysctl::sysctl_string("kern.osproductversion").unwrap_or_default(),
            os_build: sysctl::sysctl_string("kern.osversion").unwrap_or_default(),
            physical_cores: sysctl::sysctl_i32("hw.physicalcpu").unwrap_or(1).max(1) as usize,
            p_cores: sysctl::sysctl_i32("hw.perflevel0.physicalcpu")
                .filter(|v| *v > 0)
                .map(|v| v as usize),
            e_cores: sysctl::sysctl_i32("hw.perflevel1.physicalcpu")
                .filter(|v| *v > 0)
                .map(|v| v as usize),
            memory_bytes: sysctl::sysctl_u64("hw.memsize").unwrap_or(0),
            uptime_secs: sysctl::uptime_secs().unwrap_or(0),
            rosetta: sysctl::sysctl_bool("sysctl.proc_translated").unwrap_or(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_chip_and_os() {
        let info = SystemInfo::detect();
        assert!(!info.chip.is_empty());
        assert!(!info.os_version.is_empty());
        assert!(info.physical_cores > 0);
    }
}
