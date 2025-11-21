mod pwloop;
mod state;
mod utils;

use anyhow::{Context, Result};
use notify_rust::{Hint, Notification};
use state::{ActionType, Entry, State, VolumeInfo};
use tokio::sync::oneshot;
use tracing::{debug, error, info, info_span, warn};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

fn format_volume(vol: &VolumeInfo) -> String {
    let status = if vol.mute.unwrap_or(false) {
        "[MUTED]"
    } else {
        "[NOT MUTED]"
    };

    // Bad but works for debug
    // NOTE: for most real devices, volume will be in channel_volumes (as they're stereo).
    let level = match vol.volume {
        Some(v) => format!("{v}%"),
        None if !vol.channel_volumes.is_empty() => vol
            .channel_volumes
            .iter()
            .map(|v| format!("{v}%"))
            .collect::<Vec<String>>()
            .join(", "),
        None => "0%".to_string(),
    };
    format!("{level} {status}")
}

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

async fn dispatch_volume_change(
    prev_notification: Option<u32>,
    entry: &mut Entry,
    vol: VolumeInfo,
) -> Option<u32> {
    // TODO: support outputs with different volumes per channel.
    let val = vol.volume.or_else(|| {
        if vol.channel_volumes.is_empty() {
            None
        } else {
            Some(vol.channel_volumes[0])
        }
    });

    let mut n = Notification::new();
    if let Some(h) = prev_notification {
        n.id(h);
    }

    match (vol.mute, val) {
        (Some(is_muted), _) if is_muted => {
            // TODO
            let s = format!("{} - Muted", entry.get_label());
            n.summary(s.as_str());
        }
        // (Some(is_muted), _) if !is_muted => {
        //     let s = format!("{} - Unmuted", entry.get_label());
        //     n.summary(s.as_str());
        // }
        (_, Some(value)) => {
            let v = value.round() as i32;
            let s = format!("{} - {}%", entry.get_label(), v);
            n.summary(s.as_str())
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
            return prev_notification;
        }
    }

    n.urgency(notify_rust::Urgency::Normal)
        .timeout(std::time::Duration::from_secs(5))
        .show_async()
        .await
        .inspect_err(|err| error!("Failed to send notification: {err}"))
        .map(|v| v.id())
        .ok()
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
                        percent = format_volume(&vol),
                        "volume didn't change, skip"
                    );
                    return;
                }

                info!(
                    oid,
                    entry_name = e.get_label(),
                    ?vol,
                    percent = format_volume(&vol),
                    "VolumeChange"
                );

                let prev_notification = state.notification_ids.get(&oid).copied();
                match dispatch_volume_change(prev_notification, e, vol).await {
                    Some(nid) => {
                        state.notification_ids.insert(oid, nid);
                    }
                    None => {
                        state.notification_ids.remove(&oid);
                    }
                }
            }
            None => {
                warn!(oid, "got VolumeChange event for orphan device/node");
            }
        },
        ActionType::EntryRemove(oid) => match state.devices.get(&oid) {
            Some(entry) => {
                info!(oid, ?entry, "EntryRemove");
                state.remove_entry(&oid);
            }
            None => {
                warn!(oid, "got VolumeChange event for orphan device/node");
            }
        },
        ActionType::Shutdown => {
            state.clear_entries();
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
