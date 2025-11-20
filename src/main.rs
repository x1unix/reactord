mod pwloop;
mod state;
mod utils;

use anyhow::{Context, Result};
use state::{ActionType, State, VolumeInfo};
use tokio::sync::oneshot;
use tracing::{error, info, info_span, warn};
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

async fn handle_action(state: &mut State, msg: ActionType) {
    match msg {
        ActionType::EntryAdd(oid, entry) => {
            info!(oid, ?entry, "EntryAdd");
            state.devices.insert(oid, entry);
        }
        ActionType::VolumeChange(oid, vol) => match state.devices.get(&oid) {
            Some(e) => {
                info!(
                    oid,
                    entry_name = e.get_label(),
                    ?vol,
                    percent = format_volume(&vol),
                    "VolumeChange"
                );
            }
            None => {
                warn!(oid, "got VolumeChange event for orphan device/node");
            }
        },
        ActionType::EntryRemove(oid) => {
            match state.devices.get(&oid) {
                Some(entry) => {
                    // TODO
                    info!(oid, ?entry, "EntryRemove");
                }
                None => {
                    warn!(oid, "got VolumeChange event for orphan device/node");
                }
            }
        }
        ActionType::Shutdown => {
            info!("bye!");
        }
    }
}

async fn run() -> Result<()> {
    let span = info_span!("msg_listener");
    let _h = span.enter();

    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let mut h = pwloop::start_pw_thread(stop_rx).context("failed to start pipewire listener")?;
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
