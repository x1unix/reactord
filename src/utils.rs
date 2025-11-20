use std::collections::HashMap;

use crate::state;
use anyhow::{Context, Result, anyhow};
use pipewire::{self as pw, proxy::ProxyT};
use pw::{
    context::ContextRc,
    core::CoreRc,
    registry::RegistryRc,
    spa::pod::{Pod, Value, ValueArray, deserialize::PodDeserializer},
    spa::utils::dict::DictRef,
    thread_loop::ThreadLoopRc,
    types::ObjectType,
};

pub type PWContextRc = std::rc::Rc<PWContext>;

type ObjectRemoveListener = dyn Fn(u32);

pub struct Subscriptions {
    /// listeners is key-value pair of registered event listeners per object.
    /// Keeps subscriptions alive until object exists.
    listeners: HashMap<u32, Vec<Box<dyn pw::proxy::Listener>>>,

    /// Registry of PipeWire objects to keep alive.
    objects: HashMap<u32, Box<dyn pw::proxy::ProxyT>>,

    /// Object destroy listeners.
    disposers: HashMap<u32, Vec<Box<ObjectRemoveListener>>>,
}

impl Subscriptions {
    fn new() -> Self {
        Self {
            listeners: HashMap::new(),
            objects: HashMap::new(),
            disposers: HashMap::new(),
        }
    }

    fn add_subscription(&mut self, oid: u32, listener: Box<dyn pw::proxy::Listener>) {
        self.listeners.entry(oid).or_default().push(listener);
    }

    fn on_object_remove(&mut self, oid: u32, listener: Box<ObjectRemoveListener>) -> Result<()> {
        match self.objects.contains_key(&oid) {
            true => {
                self.disposers.entry(oid).or_default().push(listener);
                Ok(())
            }
            false => Err(anyhow!("object {oid} is not registered")),
        }
    }

    fn add_object(&mut self, obj: Box<dyn pw::proxy::ProxyT>) {
        let oid = obj.upcast_ref().id();
        self.objects.entry(oid).or_insert(obj);
    }

    fn remove_object(&mut self, oid: u32) {
        if let Some(subs) = self.disposers.get(&oid) {
            for sub in subs {
                sub(oid);
            }
        }

        self.disposers.remove(&oid);
        self.objects.remove(&oid);
        self.listeners.remove(&oid);
    }

    fn clear(&mut self) {
        self.disposers.clear();
        self.listeners.clear();

        // TODO: investigate why this cause 'impl_ext_end_proxy called from wrong context, check thread and locking: Operation not permitted'.
        // self.objects.clear();
    }
}

/// PWContext holds all core PipeWire objects.
pub struct PWContext {
    pub context: ContextRc,
    pub core: CoreRc,
    pub registry: RegistryRc,

    thread_loop: ThreadLoopRc,
    subs: std::rc::Rc<std::cell::RefCell<Subscriptions>>,
}

impl PWContext {
    pub fn new() -> Result<Self> {
        let tloop = new_thread_loop().context("can't create thread loop")?;
        let ctx = ContextRc::new(&tloop, None).context("can't create pw context")?;
        let core = ctx
            .connect_rc(None)
            .context("can't connect to pw context")?;
        let registry = core.get_registry_rc().context("can't get pw registry")?;
        Ok(Self {
            thread_loop: tloop,
            context: ctx,
            core,
            registry,
            subs: std::rc::Rc::new(std::cell::RefCell::new(Subscriptions::new())),
        })
    }

    pub fn new_shared() -> Result<PWContextRc> {
        let ctx = Self::new()?;
        Ok(std::rc::Rc::new(ctx))
    }

    /// Adds a new node event listener.
    /// Returns object ID that can be later used to subscribe to remove events.
    pub fn node_listener_local<F>(&self, node: pw::node::Node, builder: F) -> u32
    where
        F: Fn(u32, pw::node::NodeListenerLocalBuilder) -> pw::node::NodeListenerLocalBuilder,
    {
        let oid = node.upcast_ref().id();
        let listener = Box::new(builder(oid, node.add_listener_local()).register());
        self.subs.borrow_mut().add_subscription(oid, listener);

        let proxy: Box<dyn ProxyT> = Box::new(node);
        self.register_object(oid, proxy);
        oid
    }

    /// Adds a new device event listener.
    /// Returns object ID that can be later used to subscribe to remove events.
    pub fn device_listener_local<F>(&self, dev: pw::device::Device, builder: F) -> u32
    where
        F: Fn(
            u32,
            pw::device::DeviceListenerLocalBuilder,
        ) -> pw::device::DeviceListenerLocalBuilder,
    {
        let oid = dev.upcast_ref().id();
        let listener = Box::new(builder(oid, dev.add_listener_local()).register());
        self.subs.borrow_mut().add_subscription(oid, listener);

        let proxy: Box<dyn ProxyT> = Box::new(dev);
        self.register_object(oid, proxy);
        oid
    }

    pub fn removed_listener(&self, oid: u32, handler: Box<dyn Fn(u32)>) -> Result<()> {
        self.subs.borrow_mut().on_object_remove(oid, handler)
    }

    fn register_object(&self, oid: u32, proxy: Box<dyn ProxyT>) {
        // Register object in keepalive list and listener to remove it.
        let subs = self.subs.clone();
        let removed_listener = proxy
            .upcast_ref()
            .add_listener_local()
            .removed(move || {
                subs.borrow_mut().remove_object(oid);
            })
            .register();

        self.subs
            .borrow_mut()
            .add_subscription(oid, Box::new(removed_listener));
        self.subs.borrow_mut().add_object(proxy);
    }

    /// Starts event loop.
    ///
    /// Shuts down event loop and removes all event listeners as soon as passed method returns.
    pub fn begin(&self, cb: impl Fn()) {
        self.thread_loop.start();

        // Run until callback completes.
        cb();

        self.subs.borrow_mut().clear();
        self.thread_loop.stop();
    }
}

pub fn new_thread_loop() -> Result<ThreadLoopRc, pipewire::Error> {
    unsafe { ThreadLoopRc::new(None, None) }
}

pub fn is_audio_node(props: &Option<&DictRef>) -> bool {
    props
        .and_then(|p| p.get(*pipewire::keys::MEDIA_CLASS))
        .map(|media_class| match media_class {
            "Audio/Sink" | "Audio/Source" | "Audio/Duplex" => true,
            "Audio/Sink/Monitor" => true, // Monitor sources for recording
            _ => false,
        })
        .unwrap_or(false)
}

pub fn is_audio_device(props: &Option<&DictRef>) -> bool {
    props
        .and_then(|p| p.get(*pipewire::keys::DEVICE_API))
        .map(|device_api| match device_api {
            "alsa" => true,   // ALSA audio devices
            "bluez5" => true, // Bluetooth audio devices
            "jack" => true,   // JACK audio devices
            "pulse" => true,  // PulseAudio devices
            _ => false,
        })
        .unwrap_or(false)
}

pub fn volume_from_pod(param: &Pod) -> Option<state::VolumeInfo> {
    // TODO: try_from ?
    let obj = param.as_object().ok()?;
    let mut vol_info = state::VolumeInfo::default();

    let mut found = false;
    for prop in obj.props() {
        let key = prop.key().0;
        let value_pod = prop.value();

        match key {
            pipewire::spa::sys::SPA_PROP_volume => {
                found = true;
                vol_info.volume = value_pod.get_float().ok();
            }
            pipewire::spa::sys::SPA_PROP_mute => {
                found = true;
                vol_info.mute = value_pod.get_bool().ok();
            }
            pipewire::spa::sys::SPA_PROP_channelVolumes => {
                if let Ok((_, Value::ValueArray(ValueArray::Float(volumes)))) =
                    PodDeserializer::deserialize_any_from(value_pod.as_bytes())
                {
                    found = true;
                    vol_info.channel_volumes = volumes;
                }
            }
            _ => {}
        }
    }

    if found {
        // HACK: for Nodes, PW Pipewire sets master volume to 1.0 and puts actual volume into
        // channel_volumes.
        match vol_info.volume {
            Some(1.0) if !vol_info.channel_volumes.is_empty() => {
                vol_info.volume = Some(vol_info.channel_volumes[0]);
            }
            None if !vol_info.channel_volumes.is_empty() => {
                vol_info.volume = Some(vol_info.channel_volumes[0]);
            }
            _ => {}
        }

        Some(vol_info)
    } else {
        None
    }
}

pub type PWGlobalObject<'a> =
    pipewire::registry::GlobalObject<&'a pipewire::spa::utils::dict::DictRef>;

pub fn parse_object(o: &PWGlobalObject) -> Option<state::Entry> {
    let props = match &o.props {
        Some(props) => props,
        None => {
            eprintln!("pw: ignore node without props: {}", o.id);
            return None;
        }
    };

    let dev = match o.type_ {
        ObjectType::Node if is_audio_node(&o.props) => state::Entry {
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
                .unwrap_or(state::DeviceKind::Unknown),
        },
        ObjectType::Device if is_audio_device(&o.props) => state::Entry {
            id: o.id,
            volume: None,
            is_node: false,
            name: props.get("device.name").map(|v| v.to_string()),
            device_id: props.get("device.id").and_then(|v| v.parse::<u32>().ok()),
            label: props
                .get("device.description")
                .or_else(|| props.get("device.name"))
                .map(|v| v.to_string()),
            description: props.get("device.description").map(|v| v.to_string()),
            kind: props
                .get("media.class")
                .map(|v| v.into())
                .unwrap_or(state::DeviceKind::Unknown),
        },
        _ => {
            // eprintln!("pw: ignore unsupported object type: {}", o.type_);
            return None;
        }
    };

    Some(dev)
}

/// Returns PipeWire object friendly name.
///
/// Usually used for logging.
pub fn format_object_label(o: &PWGlobalObject) -> String {
    let props = match &o.props {
        Some(props) => props,
        None => return format!("<{}>", o.id),
    };

    match o.type_ {
        ObjectType::Node => props
            .get("node.nick")
            .or_else(|| props.get("node.description"))
            .or_else(|| props.get("node.name"))
            .map(|v| v.to_string())
            .unwrap_or_else(|| format!("<{}>", o.id)),
        ObjectType::Device => props
            .get("device.description")
            .or_else(|| props.get("device.name"))
            .map(|v| v.to_string())
            .unwrap_or_else(|| format!("Device <{}>", o.id)),
        _ => {
            format!("<{}>", o.id)
        }
    }
}
