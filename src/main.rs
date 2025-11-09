use anyhow::{Context, Result};
use ctrlc;
use pipewire as pw;
use tokio::sync::mpsc;

use pw::{context::ContextRc, core::CoreRc, registry::RegistryRc, thread_loop::ThreadLoopRc};

fn new_thread_loop() -> Result<ThreadLoopRc, pw::Error> {
    unsafe { ThreadLoopRc::new(None, None) }
}

struct PWContext {
    thread_loop: ThreadLoopRc,
    context: ContextRc,
    core: CoreRc,
    registry: RegistryRc,
}

impl PWContext {
    fn new() -> Result<PWContext> {
        let tloop = new_thread_loop().context("can't create thread loop")?;
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

fn start_pw_thread(
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
                println!("pw: new global - {global:?}");
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

fn main() {
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
