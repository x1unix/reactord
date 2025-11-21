mod pwloop;
mod state;
mod utils;

use anyhow::{Context, Result};
use notify_rust::{Hint, Notification, NotificationHandle};
use state::{ActionType, Entry, State, VolumeInfo};
use tokio::sync::oneshot;
use tracing::{debug, error, info, info_span, warn};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

fn init_logger() {
    let env_filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(format!("{}=warn", env!("CARGO_PKG_NAME"))))
        .expect("failed to get log level from environment");

    tracing_subscriber::registry()
        .with(env_filter)
        .with(tracing_subscriber::fmt::layer().with_target(true))
        .init();
}

#[tokio::main]
async fn main() {
    init_logger();
    if let Err(err) = run().await {
        error!("Error: {err}");
    }
}

fn build_volume_notification(entry: &Entry, vol: &VolumeInfo) -> Option<Notification> {
    let val = vol.volume.or_else(|| {
        if vol.channel_volumes.is_empty() {
            None
        } else {
            Some(vol.channel_volumes[0])
        }
    });

    let mut notification = Notification::new();

    match (vol.mute, val) {
        (Some(is_muted), _) if is_muted => {
            let s = format!("{} - Muted", entry.get_label());
            notification
                .summary(s.as_str())
                .icon("audio-volume-muted-symbolic");
        }
        (_, Some(value)) => {
            let v = value.round() as i32;
            let s = format!("{} - {}%", entry.get_label(), v);
            notification
                .summary(s.as_str())
                .icon("audio-volume-high-symbolic")
                .hint(Hint::CustomInt("value".to_string(), v));
        }
        _ => {
            error!(
                is_muted = vol.mute,
                ?val,
                entry_id = entry.id,
                "can't send notification as no mute or volume info"
            );
            return None;
        }
    }

    notification
        .urgency(notify_rust::Urgency::Normal)
        .timeout(std::time::Duration::from_secs(5));

    Some(notification)
}

async fn show_volume_notification(notification: Notification) -> Option<NotificationHandle> {
    notification
        .show_async()
        .await
        .inspect_err(|err| error!("Failed to send notification: {err}"))
        .ok()
}

async fn update_volume_notification(
    mut handle: NotificationHandle,
    notification: Notification,
) -> Option<NotificationHandle> {
    tokio::task::spawn_blocking(move || {
        *handle = notification;
        handle.update();
        handle
    })
    .await
    .inspect_err(|err| error!("Failed to update notification: {err}"))
    .ok()
}

async fn close_notification(handle: NotificationHandle) {
    let _ = tokio::task::spawn_blocking(move || handle.close())
        .await
        .inspect_err(|err| error!("Failed to close notification: {err}"));
}

#[cfg(target_os = "linux")]
#[tracing::instrument(name = "handle_action", skip(state, msg))]
async fn handle_action(state: &mut State, msg: ActionType) {
    match msg {
        ActionType::EntryAdd(oid, entry) => {
            info!(oid, ?entry, "EntryAdd");
            state.devices.insert(oid, entry);
        }
        ActionType::VolumeChange(oid, vol) => match state.devices.get_mut(&oid) {
            // TODO: check if state has not changed (regression when opening pamixer).
            Some(e) if e.volume.is_none() => {
                // After initial subscribe - first message is fired immediately to send a current
                // state.
                debug!(
                    oid,
                    entry_name = e.get_label(),
                    ?vol,
                    "received volume info for first time, skip notification"
                );

                e.volume = Some(vol);
            }
            Some(e) => {
                if let Some(current) = e.volume.as_ref()
                    && current == &vol
                {
                    // skip duplicate event fired when playback/resume happens
                    info!(
                        oid,
                        entry_name = e.get_label(),
                        ?vol,
                        "volume didn't change, skip"
                    );
                    return;
                }

                info!(oid, entry_name = e.get_label(), ?vol, "VolumeChange");

                let notification = match build_volume_notification(e, &vol) {
                    Some(notification) => notification,
                    None => {
                        if let Some(handle) = state.notifications.remove(&oid) {
                            close_notification(handle).await;
                        }
                        e.volume = Some(vol);
                        return;
                    }
                };

                if let Some(handle) = state.notifications.remove(&oid) {
                    if let Some(updated) = update_volume_notification(handle, notification).await {
                        state.notifications.insert(oid, updated);
                    }
                } else if let Some(handle) = show_volume_notification(notification).await {
                    state.notifications.insert(oid, handle);
                }

                e.volume = Some(vol);
            }
            None => {
                warn!(oid, "got VolumeChange event for orphan device/node");
            }
        },
        ActionType::EntryRemove(oid) => match state.devices.get(&oid) {
            Some(entry) => {
                info!(oid, ?entry, "EntryRemove");
                if let Some(handle) = state.remove_entry(&oid) {
                    close_notification(handle).await;
                }
            }
            None => {
                warn!(oid, "got VolumeChange event for orphan device/node");
            }
        },
        ActionType::Shutdown => {
            for handle in state.clear_entries() {
                close_notification(handle).await;
            }
            info!("bye!");
        }
    }
}

#[tracing::instrument(name = "run")]
async fn run() -> Result<()> {
    let span = info_span!("msg_listener");
    let _h = span.enter();
    let mut listen_cfg = pwloop::ListenerConfig::default();
    listen_cfg.set_ignore_list(vec!["easyeffects_sink".to_string()]);

    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let mut h = pwloop::start_pw_thread(stop_rx, listen_cfg)
        .context("failed to start pipewire listener")?;
    let shutdown_signal = tokio::signal::ctrl_c();
    tokio::pin!(shutdown_signal);

    let mut state = State::default();
    loop {
        tokio::select! {
            _ = &mut shutdown_signal => {
                let _ = stop_tx.send(());
                break;
            },
            Some(msg) = h.recv() => {
                handle_action(&mut state, msg).await;
            },
        }
    }

    Ok(())
}
