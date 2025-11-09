use crate::utils;
use anyhow::{Context, Result};
use pipewire as pw;
use pw::{
    context::ContextRc, core::CoreRc, registry::RegistryRc, spa::utils::dict::DictRef,
    thread_loop::ThreadLoopRc, types::ObjectType,
};
use tokio::sync::mpsc;

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

fn print_props(otype: &ObjectType, label: &str, props: &Option<&DictRef>) {
    if let Some(kv) = props {
        let key = if *otype == ObjectType::Device {
            "device.description"
        } else {
            "node.description"
        };

        let name = kv.get(key).unwrap_or("<unnamed>");
        println!("PW:{label:<6} - {name}");

        kv.iter().for_each(|(k, v)| println!("  {k:<20} => {v}"));
        return;
    }

    println!("PW:{label}");
}

fn on_global_change(o: &PWGlobalObject) {
    match o.type_ {
        ObjectType::Node if utils::is_audio_node(&o.props) => {
            print_props(&o.type_, "Node", &o.props);
        }
        ObjectType::Device if utils::is_audio_device(&o.props) => {
            print_props(&o.type_, "Device", &o.props);
        }
        _ => {}
    }
}

pub fn start_pw_thread(
    cancel_token: std::sync::mpsc::Receiver<()>,
    _sender: Option<mpsc::UnboundedSender<()>>,
) -> Result<std::thread::JoinHandle<()>> {
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

        println!("pw: registering listener...");
        let _listener = pwctx
            .registry
            .add_listener_local()
            .global(|global| {
                on_global_change(global);
            })
            .register();

        println!("pw: starting thread loop...");
        pwctx.thread_loop.start();

        // Suspend thread until cancellation signal is sent.
        // PW's ThreadLoop already manages its own thread under the hood.
        cancel_token.recv().ok();

        println!("pw: shutting down...");
        pwctx.thread_loop.stop();
    });

    Ok(h)
}

fn _main() {
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    ctrlc::set_handler(move || {
        let _ = stop_tx.send(());
    })
    .expect("failed to init shutdown handler");

    let h = match start_pw_thread(stop_rx, None) {
        Ok(h) => h,
        Err(err) => {
            eprintln!("failed to start pipewire listener: {err}");
            return;
        }
    };

    h.join().unwrap();
    println!("bye");
}
