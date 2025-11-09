use crate::{
    state::{ActionType, DeviceKind, Entry},
    utils,
};
use anyhow::{Context, Result};
use pipewire as pw;
use pw::{
    context::ContextRc, core::CoreRc, registry::RegistryRc, spa::utils::dict::DictRef,
    thread_loop::ThreadLoopRc, types::ObjectType,
};

struct PWContext {
    thread_loop: ThreadLoopRc,
    context: ContextRc,
    core: CoreRc,
    registry: RegistryRc,
}

impl PWContext {
    fn new() -> Result<PWContext> {
        let tloop = utils::new_thread_loop().context("can't create thread loop")?;
        let ctx = ContextRc::new(&tloop, None).context("can't create pw context")?;
        let core = ctx
            .connect_rc(None)
            .context("can't connect to pw context")?;
        let registry = core.get_registry_rc().context("can't get pw registry")?;

        Ok(PWContext {
            thread_loop: tloop,
            context: ctx,
            core,
            registry,
        })
    }
}

type PWGlobalObject<'a> = pw::registry::GlobalObject<&'a pw::spa::utils::dict::DictRef>;

fn on_global_change(sender: ActionSender, o: &PWGlobalObject) {
    let entry = match parse_object(o) {
        Some(e) => e,
        None => {
            return;
        }
    };

    if let Err(err) = sender.send(ActionType::EntryAdd(entry)) {
        eprintln!(
            "pw: failed to dispatch EntryAdd({:?} {}): {err}",
            o.type_, o.id,
        );
    }
}

fn parse_object(o: &PWGlobalObject) -> Option<Entry> {
    let props = match &o.props {
        Some(props) => props,
        None => {
            eprintln!("pw: ignore node without props: {}", o.id);
            return None;
        }
    };

    let dev = match o.type_ {
        ObjectType::Node if utils::is_audio_node(&o.props) => Entry {
            id: o.id,
            volume: None,
            is_node: true,
            name: props.get("node.name").map(|v| v.to_string()),
            device_id: props.get("device.id").and_then(|v| v.parse::<u32>().ok()),
            label: props
                .get("node.nick")
                .or_else(|| props.get("node.description"))
                .map(|v| v.to_string()),
            description: props.get("node.description").map(|v| v.to_string()),
            kind: props
                .get("media.class")
                .map(|v| v.into())
                .unwrap_or(DeviceKind::Unknown),
        },
        ObjectType::Device if utils::is_audio_device(&o.props) => Entry {
            id: o.id,
            volume: None,
            is_node: false,
            name: props.get("device.name").map(|v| v.to_string()),
            device_id: props.get("device.id").and_then(|v| v.parse::<u32>().ok()),
            label: props
                .get("device.descriotion")
                .or_else(|| props.get("device.name"))
                .map(|v| v.to_string()),
            description: props.get("device.description").map(|v| v.to_string()),
            kind: props
                .get("media.class")
                .map(|v| v.into())
                .unwrap_or(DeviceKind::Unknown),
        },
        _ => {
            eprintln!("pw: ignore unsupported object type: {}", o.type_);
            return None;
        }
    };

    Some(dev)
}

type ActionSender = std::sync::mpsc::SyncSender<ActionType>;
type ActionListener = std::sync::mpsc::Receiver<ActionType>;

pub fn start_pw_thread(cancel_token: std::sync::mpsc::Receiver<()>) -> Result<ActionListener> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<ActionType>(3);

    let h = std::thread::spawn(move || {
        println!("pw: initializing...");
        pw::init();
        println!("pw: initialized");

        let pwctx = match PWContext::new() {
            Ok(r) => r,
            Err(err) => {
                eprintln!("failed to build pipewire consumer: {err}");
                return;
            }
        };

        let sent_tx = tx.clone();
        println!("pw: registering listener...");
        let _listener = pwctx
            .registry
            .add_listener_local()
            .global(move |global| {
                on_global_change(sent_tx, global);
            })
            .register();

        println!("pw: starting thread loop...");
        pwctx.thread_loop.start();

        // Suspend thread until cancellation signal is sent.
        // PW's ThreadLoop already manages its own thread under the hood.
        cancel_token.recv().ok();

        println!("pw: shutting down...");
        pwctx.thread_loop.stop();
        let _ = tx.send(ActionType::Shutdown);
    });

    Ok(rx)
}
