use crate::{state::ActionType, utils};
use anyhow::{Context, Result};
use pipewire as pw;
use pw::{context::ContextRc, core::CoreRc, registry::RegistryRc, thread_loop::ThreadLoopRc};

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

fn on_global_change(sender: ActionSender, o: &utils::PWGlobalObject) {
    let entry = match utils::parse_object(o) {
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
                on_global_change(sent_tx.clone(), global);
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
