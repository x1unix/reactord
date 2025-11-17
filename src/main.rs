mod pwloop;
mod state;
mod utils;

use anyhow::Result;
use state::{ActionType, VolumeInfo};

fn new_cancel_token() -> Result<std::sync::mpsc::Receiver<()>> {
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    ctrlc::set_handler(move || {
        let _ = stop_tx.send(());
    })?;

    Ok(stop_rx)
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

    for msg in h {
        match msg {
            ActionType::EntryAdd(oid, e) => {
                println!(
                    "#{oid} PW:{:?} - {}",
                    e.kind,
                    e.label.as_deref().unwrap_or("<unnamed>")
                )
            }
            ActionType::Shutdown => {
                println!("bye!");
                return;
            }
            e => {
                // TODO
                println!("Event: {e:?}");
            }
        }
    }
}
