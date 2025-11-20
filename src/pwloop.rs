use crate::{state::ActionType, utils};
use anyhow::{Context, Result};
use pipewire::{self as pw, proxy::ProxyT, spa::param::ParamType};
use pw::types::ObjectType;
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
                    let _ = vol_sender.send(ActionType::VolumeChange(dev_id, vol));
                }
            })
        }),
        Box::new(move |oid: u32| {
            let span = tracing::debug_span!("device_removed", node_id = oid,);
            let _g = span.enter();

            debug!(%oid, "device removed");
            if let Err(err) = rm_sender.send(ActionType::EntryRemove(oid)) {
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

                if param_type != ParamType::Props {
                    return;
                }

                // TODO: support other prop change events?
                if let Some(vol) = param.and_then(utils::volume_from_pod) {
                    debug!(%node_id, volume = ?vol, "node volume change");
                    let _ = vol_sender.send(ActionType::VolumeChange(node_id, vol));
                }
            })
        }),
        Box::new(move |oid: u32| {
            let span = tracing::debug_span!("node_removed", node_id = oid,);
            let _g = span.enter();

            debug!(%oid, "node removed");
            if let Err(err) = rm_sender.send(ActionType::EntryRemove(oid)) {
                error!(%oid, ?err, "failed to dispatch EntryRemove");
            }
        }),
    )?;
    Ok(())
}

#[tracing::instrument(
    name = "global_change",
    skip(ctx, sender, o),
    fields(obj_id = o.id),
)]
fn on_global_change(ctx: PWContextRc, sender: ActionSender, o: &PWGlobalObject) -> Result<()> {
    let entry = match utils::parse_object(o) {
        Some(e) => e,
        None => {
            return Ok(());
        }
    };

    let label = utils::format_object_label(o);
    match o.type_ {
        ObjectType::Node if utils::is_audio_node(&o.props) => {
            let node: pw::node::Node = ctx
                .registry
                .bind(o)
                .with_context(|| format!("failed to bind node #{}", &label))?;

            let node_id = node.upcast_ref().id();
            debug!(node_id, label = &label, "new node");
            if let Err(err) = sender.send(ActionType::EntryAdd(node_id, entry)) {
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
            if let Err(err) = sender.send(ActionType::EntryAdd(dev_id, entry)) {
                error!(dev_id, label = &label, "failed to dispatch EntryAdd: {err}");
            }

            subscribe_device(ctx, sender, dev)?;
        }
        _ => {}
    };

    Ok(())
}

type ActionSender = std::sync::mpsc::SyncSender<ActionType>;
type ActionListener = std::sync::mpsc::Receiver<ActionType>;

pub fn start_pw_thread(cancel_token: std::sync::mpsc::Receiver<()>) -> Result<ActionListener> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<ActionType>(3);

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

        debug!("registering listener...");
        let _listener = pwctx
            .registry
            .add_listener_local()
            .global(move |global| {
                if let Err(err) = on_global_change(cctx.clone(), sent_tx.clone(), global) {
                    error!("on global change hook returned an error: {err}");
                }
            })
            .register();

        debug!("starting thread loop...");
        pwctx.begin(|| {
            // Suspend thread until cancellation signal is sent.
            // PW's ThreadLoop already manages its own thread under the hood.
            cancel_token.recv().ok();
            info!("shutting down...");
        });

        let _ = tx.send(ActionType::Shutdown);
    });

    Ok(rx)
}
