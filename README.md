# ReactorD

**Reactord** is background daemon which listens PipeWire events and displays desktop notifications.\
Main goal is to track PipeWire device or node volume change and show a notification.

## Infrastructure

- `main.rs` - Main thread with Tokio async to receive incoming events and react on them.
- `pwloop.rs` - Pipewire event listener. Runs on a separate, isolated thread (as this is required by _pipewire_ crate) and routes events to `main.rs` using mpsc channel.
