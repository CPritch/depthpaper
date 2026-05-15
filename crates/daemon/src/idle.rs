use tracing::{debug, info, warn};
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{BindError, GlobalList},
    protocol::wl_seat,
};
use wayland_protocols::ext::idle_notify::v1::client::{
    ext_idle_notification_v1::{self, ExtIdleNotificationV1},
    ext_idle_notifier_v1::ExtIdleNotifierV1,
};

use crate::wayland::App;

pub struct IdleState {
    pub idle: bool,
    _notifier: ExtIdleNotifierV1,
    _notification: ExtIdleNotificationV1,
}

impl IdleState {
    pub fn bind(
        globals: &GlobalList,
        qh: &QueueHandle<App>,
        seat: &wl_seat::WlSeat,
        timeout_secs: u64,
    ) -> Result<Self, BindError> {
        let notifier: ExtIdleNotifierV1 = globals.bind(qh, 1..=1, ())?;
        let timeout_ms = (timeout_secs * 1000).min(u32::MAX as u64) as u32;
        let notification = notifier.get_idle_notification(timeout_ms, seat, qh, ());
        info!(timeout_secs, "ext-idle-notify-v1 bound");
        Ok(Self {
            idle: false,
            _notifier: notifier,
            _notification: notification,
        })
    }
}

impl Dispatch<ExtIdleNotifierV1, ()> for App {
    fn event(
        _: &mut Self,
        _: &ExtIdleNotifierV1,
        _: <ExtIdleNotifierV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Notifier has no events.
    }
}

impl Dispatch<ExtIdleNotificationV1, ()> for App {
    fn event(
        state: &mut Self,
        _: &ExtIdleNotificationV1,
        event: <ExtIdleNotificationV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            ext_idle_notification_v1::Event::Idled => {
                debug!("session idled");
                if let Some(idle) = &mut state.idle {
                    idle.idle = true;
                }
            }
            ext_idle_notification_v1::Event::Resumed => {
                debug!("session resumed");
                if let Some(idle) = &mut state.idle {
                    idle.idle = false;
                }
                state.needs_render = true;
            }
            _ => {}
        }
    }
}

pub fn try_bind(
    globals: &GlobalList,
    qh: &QueueHandle<App>,
    seat: Option<&wl_seat::WlSeat>,
    timeout_secs: u64,
) -> Option<IdleState> {
    let seat = match seat {
        Some(s) => s,
        None => {
            warn!("no seat available, idle notifications disabled");
            return None;
        }
    };
    match IdleState::bind(globals, qh, seat, timeout_secs) {
        Ok(s) => Some(s),
        Err(e) => {
            warn!("ext-idle-notify-v1 unavailable, idle detection disabled: {e}");
            None
        }
    }
}
