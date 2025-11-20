mod pwloop;
mod state;
mod utils;

use anyhow::{Context, Result};
use state::{ActionType, State, VolumeInfo};
use tracing::{debug, error, info, info_span, warn};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

fn new_cancel_token() -> Result<std::sync::mpsc::Receiver<()>> {
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    ctrlc::set_handler(move || {
        let _ = stop_tx.send(());
    })?;

    Ok(stop_rx)
}

fn format_volume(vol: &VolumeInfo) -> String {
    let status = if vol.mute.unwrap_or(false) {
        "[MUTED]"
    } else {
        "[NOT MUTED]"
    };

    // Bad but works for debug
    // NOTE: for most real devices, volume will be in channel_volumes (as they're stereo).
    let level = match vol.volume {
        // FIXME: why volume here is always empty?
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

fn main() {
    init_logger();
    if let Err(err) = run() {
        eprintln!("Error: {err}");
    }
}

fn run() -> Result<()> {
    let stop_rx = new_cancel_token().context("failed to init shutdown handler")?;
    let h = pwloop::start_pw_thread(stop_rx).context("failed to start pipewire listener")?;
    let mut state = State::default();

    let span = info_span!("msg_listener");
    let _h = span.enter();

    for msg in h {
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
                        continue;
                    }
                    None => {
                        warn!(oid, "got VolumeChange event for orphan device/node");
                    }
                }
            }
            ActionType::Shutdown => {
                info!("bye!");
                break;
            }
        }
    }

    Ok(())
}
