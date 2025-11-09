use anyhow::{Context, Result};
use pipewire as pw;
use tokio::sync::mpsc;

use pw::thread_loop::ThreadLoopRc;

fn new_thread_loop() -> Result<ThreadLoopRc, pw::Error> {
    unsafe { ThreadLoopRc::new(None, None) }
}

struct PWContext<'a> {
    thread_loop: ThreadLoopRc,
    context: pw::context::ContextBox<'a>,
    core: pw::core::CoreBox<'a>,
    registry: pw::registry::RegistryBox<'a>,
}

impl<'a> PWContext<'a> {
    fn new() -> Result<PWContext<'a>> {
        pw::init();
        let tloop = new_thread_loop().context("can't create thread loop")?;
        let ctx = pw::context::ContextBox::new(&tloop.loop_(), None)
            .context("can't create pw context")?;
        let core = ctx.connect(None).context("can't connect to pw context")?;
        let registry = core.get_registry().context("can't get pw registry")?;

        Ok(PWContext{
            thread_loop = tloop,
            context = ctx,
            core = core,
            registry = registry
        })
    }
}

fn start_pw_thread(sender: mpsc::UnboundedSender<()>) -> Result<()> {
    std::thread::spawn(move || {
        println!("pw: starting...");
        pw::init();
        println!("pw: started");
    });

    Ok(())
}

fn main() {
    println!("Hello, world!");
}
