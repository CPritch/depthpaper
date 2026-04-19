use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy)]
pub struct BatteryState {
    pub capacity: u8,
    pub discharging: bool,
}

/// Read the first BAT* entry from /sys/class/power_supply.
/// Returns None on a desktop or any read failure — callers treat that
/// as "no battery gating", which is the right default.
pub fn read() -> Option<BatteryState> {
    let entries = fs::read_dir("/sys/class/power_supply").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with("BAT") {
            return read_one(entry.path());
        }
    }
    None
}

fn read_one(dir: PathBuf) -> Option<BatteryState> {
    let capacity: u8 = fs::read_to_string(dir.join("capacity"))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    let status = fs::read_to_string(dir.join("status")).ok()?;
    Some(BatteryState {
        capacity,
        discharging: status.trim() == "Discharging",
    })
}

/// Return true if rendering should be permitted given the policy.
/// `threshold == 0` disables battery gating entirely.
pub fn render_allowed(state: Option<BatteryState>, threshold: u8) -> bool {
    if threshold == 0 {
        return true;
    }
    match state {
        Some(s) if s.discharging && s.capacity < threshold => false,
        _ => true,
    }
}