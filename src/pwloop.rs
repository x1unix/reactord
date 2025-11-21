use std::rc::Rc;

use crate::{
    state::{ActionType, Entry},
    utils,
};
use anyhow::{Context, Result};
use pipewire::{self as pw, proxy::ProxyT, spa::param::ParamType};
use pw::types::ObjectType;
use tokio::sync::oneshot;
use tracing::{debug, debug_span, error, info};
use utils::{PWContext, PWContextRc, PWGlobalObject};

#[tracing::instrument(
    name = "subscribe_device",
    skip(ctx, sender, dev),
    fields(device_id = dev.upcast_ref().id()),
)]
fn subscribe_device(ctx: PWContextRc, sender: ActionSender, dev: pw::device::Device) -> Result<()> {
    dev.subscribe_params(&[
        pw::spa::param::ParamType::Props,
        pw::spa::param::ParamType::Route,
    ]);

    let rm_sender = sender.clone();
    ctx.removed_listener(
        ctx.device_listener_local(dev, move |dev_id, b| {
            let vol_sender = sender.clone();
            b.param(move |_seq, param_type, _idx, _next, param| {
                let span = debug_span!("device_listener", dev_id);
                let _g = span.enter();

                if param_type != ParamType::Props {
                    return;
                }

                // TODO: support other prop change events?
                if let Some(vol) = param.and_then(utils::volume_from_pod) {
                    debug!(%dev_id, volume = ?vol, "device volume change");
                    let _ = vol_sender.blocking_send(ActionType::VolumeChange(dev_id, vol));
                }
            })
        }),
        Box::new(move |oid: u32| {
            let span = tracing::debug_span!("device_removed", node_id = oid,);
            let _g = span.enter();

            debug!(%oid, "device removed");
            if let Err(err) = rm_sender.blocking_send(ActionType::EntryRemove(oid)) {
                error!(%oid, ?err, "failed to dispatch EntryRemove");
            }
        }),
    )?;
    Ok(())
}

#[tracing::instrument(
    name = "subscribe_node",
    skip(ctx, sender, node),
    fields(node_id = node.upcast_ref().id()),
)]
fn subscribe_node(ctx: PWContextRc, sender: ActionSender, node: pw::node::Node) -> Result<()> {
    node.subscribe_params(&[
        pw::spa::param::ParamType::Props,
        pw::spa::param::ParamType::Route,
    ]);

    let rm_sender = sender.clone();
    ctx.removed_listener(
        ctx.node_listener_local(node, move |node_id, b| {
            let vol_sender = sender.clone();
            b.param(move |_seq, param_type, _idx, _next, param| {
                let span = debug_span!("node_listener", node_id);
                let _g = span.enter();

                match param_type {
                    ParamType::Props => {
                        if let Some(vol) = param.and_then(utils::volume_from_pod) {
                            debug!(%node_id, volume = ?vol, "node volume change");
                            let _ =
                                vol_sender.blocking_send(ActionType::VolumeChange(node_id, vol));
                        }
                    }
                    _ => {
                        debug!(?param_type, "skip unsupported node param type");
                    }
                }
            })
        }),
        Box::new(move |oid: u32| {
            let span = tracing::debug_span!("node_removed", node_id = oid,);
            let _g = span.enter();

            debug!(%oid, "node removed");
            if let Err(err) = rm_sender.blocking_send(ActionType::EntryRemove(oid)) {
                error!(%oid, ?err, "failed to dispatch EntryRemove");
            }
        }),
    )?;
    Ok(())
}

#[tracing::instrument(
    name = "global_change",
    skip(ctx, cfg, sender, o),
    fields(obj_id = o.id),
)]
fn on_global_change(
    ctx: PWContextRc,
    cfg: std::rc::Rc<ListenerConfig>,
    sender: ActionSender,
    o: &PWGlobalObject,
) -> Result<()> {
    let entry = match utils::parse_object(o) {
        Some(e) => e,
        None => {
            return Ok(());
        }
    };

    let label = utils::format_object_label(o);
    if cfg.is_entry_ignored(&entry) {
        debug!(label = &label, "skip ignored entry");
        return Ok(());
    }

    match o.type_ {
        ObjectType::Node if utils::is_audio_node(&o.props) => {
            let node: pw::node::Node = ctx
                .registry
                .bind(o)
                .with_context(|| format!("failed to bind node #{}", &label))?;

            let node_id = node.upcast_ref().id();
            debug!(node_id, label = &label, "new node");
            if let Err(err) = sender.blocking_send(ActionType::EntryAdd(node_id, entry)) {
                error!(
                    node_id,
                    label = &label,
                    "failed to dispatch EntryAdd: {err}"
                );
            }

            subscribe_node(ctx, sender, node)?;
        }
        ObjectType::Device if utils::is_audio_device(&o.props) => {
            let dev: pw::device::Device = ctx.registry.bind(o).with_context(|| {
                format!("failed to bind device {}", utils::format_object_label(o))
            })?;

            let dev_id = dev.upcast_ref().id();
            debug!(dev_id, label = &label, "new device");
            if let Err(err) = sender.blocking_send(ActionType::EntryAdd(dev_id, entry)) {
                error!(dev_id, label = &label, "failed to dispatch EntryAdd: {err}");
            }

            subscribe_device(ctx, sender, dev)?;
        }
        _ => {}
    };

    Ok(())
}

type ActionSender = tokio::sync::mpsc::Sender<ActionType>;
type ActionListener = tokio::sync::mpsc::Receiver<ActionType>;

pub struct ListenerConfig {
    message_buffer_size: usize,
    ignore_list: Option<std::collections::HashSet<String>>,
}

impl Default for ListenerConfig {
    fn default() -> Self {
        Self {
            message_buffer_size: 5,
            ignore_list: Default::default(),
        }
    }
}

impl ListenerConfig {
    #[allow(dead_code)]
    pub fn set_message_buffer_size(&mut self, s: usize) {
        self.message_buffer_size = s;
    }

    #[allow(dead_code)]
    pub fn set_ignore_list(&mut self, ignore_list: Vec<String>) {
        self.ignore_list = if ignore_list.is_empty() {
            None
        } else {
            Some(ignore_list.into_iter().collect())
        };
    }

    fn is_entry_ignored(&self, e: &Entry) -> bool {
        match self.ignore_list.as_ref() {
            Some(ignore_list) => e
                .name
                .as_ref()
                .or(e.label.as_ref())
                .map(|v| ignore_list.contains(v))
                .unwrap_or(false),
            None => false,
        }
    }
}

/// Starts a separate thread to listen for PipeWire events.
/// Thread is terminated as soon as a new message received from a cancellation token channel.
///
/// Returns event channel to listen for incoming events.
pub fn start_pw_thread(
    cancel_token: oneshot::Receiver<()>,
    cfg: ListenerConfig,
) -> Result<ActionListener> {
    let (tx, rx) = tokio::sync::mpsc::channel::<ActionType>(cfg.message_buffer_size);

    let _h = std::thread::spawn(move || {
        let span = tracing::info_span!("pw");
        let _h = span.enter();

        debug!("initializing pipewire");
        pw::init();
        debug!("initialized");

        let pwctx = match PWContext::new_shared() {
            Ok(r) => r,
            Err(err) => {
                error!("failed to build pipewire consumer: {err}");
                return;
            }
        };

        // refcounters to be passed to the callback.
        let cctx = pwctx.clone();
        let sent_tx = tx.clone();

        let cfg_rc = Rc::new(cfg);
        debug!("registering listener...");
        let _listener = pwctx
            .registry
            .add_listener_local()
            .global(move |global| {
                if let Err(err) =
                    on_global_change(cctx.clone(), cfg_rc.clone(), sent_tx.clone(), global)
                {
                    error!("on global change hook returned an error: {err}");
                }
            })
            .register();

        debug!("starting thread loop...");
        pwctx.begin(|| {
            // Suspend thread until cancellation signal is sent.
            // PW's ThreadLoop already manages its own thread under the hood.
            cancel_token.blocking_recv().ok();
            info!("shutting down...");
        });

        let _ = tx.blocking_send(ActionType::Shutdown);
    });

    Ok(rx)
}
