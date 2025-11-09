mod pwloop;
mod utils;

use anyhow::Result;

fn new_cancel_token() -> Result<std::sync::mpsc::Receiver<()>> {
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    ctrlc::set_handler(move || {
        let _ = stop_tx.send(());
    })?;

    Ok(stop_rx)
}

fn main() {
    let stop_rx = new_cancel_token().expect("failed to init shutdown handler");
    let h = match pwloop::start_pw_thread(stop_rx, None) {
        Ok(h) => h,
        Err(err) => {
            eprintln!("failed to start pipewire listener: {err}");
            return;
        }
    };

    h.join().unwrap();
    println!("bye");
}
