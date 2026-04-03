use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Smoothed cursor state with configurable poll rate.
pub struct CursorPoller {
    socket_path: PathBuf,
    poll_interval: Duration,
    last_poll: Instant,
    /// Raw absolute cursor position from Hyprland.
    pub raw_x: f32,
    pub raw_y: f32,
    /// Smoothed cursor offset normalized to [-0.5, 0.5] per-monitor.
    pub offset_x: f32,
    pub offset_y: f32,
    /// Whether the cursor moved since last poll.
    pub changed: bool,
}

impl CursorPoller {
    pub fn new(poll_hz: u32) -> Option<Self> {
        let socket_path = hyprland_socket_path()?;
        Some(Self {
            socket_path,
            poll_interval: Duration::from_secs_f64(1.0 / poll_hz as f64),
            last_poll: Instant::now() - Duration::from_secs(1), // force immediate first poll
            raw_x: 0.0,
            raw_y: 0.0,
            offset_x: 0.0,
            offset_y: 0.0,
            changed: false,
        })
    }

    /// Poll Hyprland for cursor position if enough time has elapsed.
    /// `monitor_x`, `monitor_y` are the monitor's top-left in global coords.
    /// `monitor_w`, `monitor_h` are the monitor's dimensions.
    /// Returns true if cursor moved.
    pub fn poll(
        &mut self,
        monitor_x: f32,
        monitor_y: f32,
        monitor_w: f32,
        monitor_h: f32,
        smoothing: f32,
    ) -> bool {
        if self.last_poll.elapsed() < self.poll_interval {
            return false;
        }
        self.last_poll = Instant::now();

        let (x, y) = match query_cursor_pos(&self.socket_path) {
            Some(pos) => pos,
            None => return false,
        };

        self.raw_x = x;
        self.raw_y = y;

        // Normalize cursor position relative to monitor center → [-0.5, 0.5]
        let target_x = (x - monitor_x) / monitor_w - 0.5;
        let target_y = (y - monitor_y) / monitor_h - 0.5;

        let prev_x = self.offset_x;
        let prev_y = self.offset_y;

        // Lerp for smooth movement
        self.offset_x += (target_x - self.offset_x) * smoothing;
        self.offset_y += (target_y - self.offset_y) * smoothing;

        // Consider "changed" if offset moved by more than a tiny epsilon
        self.changed = (self.offset_x - prev_x).abs() > 1e-5
            || (self.offset_y - prev_y).abs() > 1e-5;

        self.changed
    }
}

fn hyprland_socket_path() -> Option<PathBuf> {
    let runtime_dir = std::env::var("XDG_RUNTIME_DIR").ok()?;
    let instance_sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    let path = PathBuf::from(runtime_dir)
        .join("hypr")
        .join(instance_sig)
        .join(".socket.sock");
    if path.exists() {
        Some(path)
    } else {
        tracing::warn!(?path, "Hyprland socket not found");
        None
    }
}

fn query_cursor_pos(socket_path: &PathBuf) -> Option<(f32, f32)> {
    let mut stream = UnixStream::connect(socket_path).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(50))).ok()?;
    stream.write_all(b"cursorpos").ok()?;

    let mut buf = [0u8; 64];
    let n = stream.read(&mut buf).ok()?;
    let response = std::str::from_utf8(&buf[..n]).ok()?.trim();

    // Response format: "X, Y"
    let mut parts = response.split(',');
    let x: f32 = parts.next()?.trim().parse().ok()?;
    let y: f32 = parts.next()?.trim().parse().ok()?;
    Some((x, y))
}