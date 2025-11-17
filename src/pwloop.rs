use crate::{state::ActionType, utils};
use anyhow::{Context, Result};
use pipewire::{self as pw, proxy::ProxyT, spa::param::ParamType};
use pw::types::ObjectType;
use utils::{PWContext, PWContextRc, PWGlobalObject};

fn subscribe_device(ctx: PWContextRc, sender: ActionSender, dev: pw::device::Device) -> Result<()> {
    dev.subscribe_params(&[
        pw::spa::param::ParamType::Props,
        pw::spa::param::ParamType::Route,
    ]);

    ctx.removed_listener(
        ctx.device_listener_local(dev, move |oid, b| {
            b.param(move |_seq, param_type, _idx, _next, param| {
                if param_type != ParamType::Props {
                    return;
                }

                // TODO: support other prop change events?
                if let Some(vol) = param.and_then(utils::volume_from_pod) {
                    println!("pw: volume event: Object=Device; Id={oid} Vol={vol:?};");
                }
            })
        }),
        Box::new(move |oid: u32| {
            if let Err(err) = sender.send(ActionType::EntryRemove(oid)) {
                eprintln!("pw: failed to dispatch EntryRemove({oid}): {err}",);
            }
        }),
    )?;
    Ok(())
}

fn subscribe_node(ctx: PWContextRc, sender: ActionSender, node: pw::node::Node) -> Result<()> {
    node.subscribe_params(&[
        pw::spa::param::ParamType::Props,
        pw::spa::param::ParamType::Route,
    ]);
    ctx.removed_listener(
        ctx.node_listener_local(node, move |oid, b| {
            b.param(move |_seq, param_type, _idx, _next, param| {
                if param_type != ParamType::Props {
                    return;
                }

                // TODO: support other prop change events?
                if let Some(vol) = param.and_then(utils::volume_from_pod) {
                    println!("pw: volume event: Object=Node; Id={oid} Vol={vol:?};");
                }
            })
        }),
        Box::new(move |oid: u32| {
            if let Err(err) = sender.send(ActionType::EntryRemove(oid)) {
                eprintln!("pw: failed to dispatch EntryRemove({oid}): {err}",);
            }
        }),
    )?;
    Ok(())
}

fn on_global_change(ctx: PWContextRc, sender: ActionSender, o: &PWGlobalObject) -> Result<()> {
    let entry = match utils::parse_object(o) {
        Some(e) => e,
        None => {
            return Ok(());
        }
    };

    println!("pw: subscribe to {}", entry.get_label());

    match o.type_ {
        ObjectType::Node if utils::is_audio_node(&o.props) => {
            let node: pw::node::Node = ctx.registry.bind(o).with_context(|| {
                format!("failed to bind node #{}", utils::format_object_label(o))
            })?;

            let oid = node.upcast_ref().id();
            if let Err(err) = sender.send(ActionType::EntryAdd(oid, entry)) {
                eprintln!(
                    "pw: failed to dispatch EntryAdd({:?} {}): {err}",
                    o.type_, o.id,
                );
            }

            subscribe_node(ctx, sender, node)?;
        }
        ObjectType::Device if utils::is_audio_device(&o.props) => {
            let dev: pw::device::Device = ctx.registry.bind(o).with_context(|| {
                format!("failed to bind device {}", utils::format_object_label(o))
            })?;

            let oid = dev.upcast_ref().id();
            if let Err(err) = sender.send(ActionType::EntryAdd(oid, entry)) {
                eprintln!(
                    "pw: failed to dispatch EntryAdd({:?} {}): {err}",
                    o.type_, o.id,
                );
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
        println!("pw: initializing...");
        pw::init();
        println!("pw: initialized");

        let pwctx = match PWContext::new_shared() {
            Ok(r) => r,
            Err(err) => {
                eprintln!("failed to build pipewire consumer: {err}");
                return;
            }
        };

        // refcounters to be passed to the callback.
        let cctx = pwctx.clone();
        let sent_tx = tx.clone();

        println!("pw: registering listener...");
        let _listener = pwctx
            .registry
            .add_listener_local()
            .global(move |global| {
                if let Err(err) = on_global_change(cctx.clone(), sent_tx.clone(), global) {
                    println!("pw:Error - {err}");
                }
            })
            .register();

        println!("pw: starting thread loop...");
        pwctx.begin(|| {
            // Suspend thread until cancellation signal is sent.
            // PW's ThreadLoop already manages its own thread under the hood.
            cancel_token.recv().ok();
            println!("pw: shutting down...");
        });

        let _ = tx.send(ActionType::Shutdown);
    });

    Ok(rx)
}
