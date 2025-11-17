mod pwloop;
mod state;
mod utils;

use anyhow::Result;
use state::{ActionType, State, VolumeInfo};

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
    let level = match vol.volume {
        // FIXME: why volume here is always empty?
        Some(v) => format!("{}%", v * 100.0),
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

fn main() {
    let stop_rx = new_cancel_token().expect("failed to init shutdown handler");
    let h = match pwloop::start_pw_thread(stop_rx) {
        Ok(h) => h,
        Err(err) => {
            eprintln!("failed to start pipewire listener: {err}");
            return;
        }
    };

    let mut state = State::default();

    for msg in h {
        match msg {
            ActionType::EntryAdd(oid, e) => {
                println!("#{oid} PW:{:?} - {}", e.kind, e.get_label());
                state.devices.insert(oid, e);
            }
            ActionType::VolumeChange(oid, vol) => {
                match state.devices.get(&oid) {
                    Some(e) => {
                        // TODO
                        println!(
                            "#{oid} PW:{:?} {} - {}",
                            e.kind,
                            e.get_label(),
                            format_volume(&vol)
                        );
                    }
                    None => {
                        println!("Warn: got VolumeChange event for orphan device/node {oid}");
                    }
                }
            }
            ActionType::EntryRemove(oid) => {
                match state.devices.get(&oid) {
                    Some(e) => {
                        // TODO
                        println!("#{oid} PW:{:?} {} - Disconnected", e.kind, e.get_label(),);
                    }
                    None => {
                        println!("Warn: got EntryRemove event for orphan device/node {oid}");
                    }
                }
            }
            ActionType::Shutdown => {
                println!("bye!");
                return;
            }
        }
    }
}
