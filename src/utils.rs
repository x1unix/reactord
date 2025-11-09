use crate::state;
use pipewire::{
    spa::pod::{Pod, Value, ValueArray, deserialize::PodDeserializer},
    spa::utils::dict::DictRef,
    thread_loop::ThreadLoopRc,
    types::ObjectType,
};

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

    for prop in obj.props() {
        let key = prop.key().0;
        let value_pod = prop.value();

        match key {
            pipewire::spa::sys::SPA_PROP_volume => {
                vol_info.volume = value_pod.get_float().ok();
            }
            pipewire::spa::sys::SPA_PROP_mute => {
                vol_info.mute = value_pod.get_bool().ok();
            }
            pipewire::spa::sys::SPA_PROP_channelVolumes => {
                if let Ok((_, Value::ValueArray(ValueArray::Float(volumes)))) =
                    PodDeserializer::deserialize_any_from(value_pod.as_bytes())
                {
                    vol_info.channel_volumes = volumes;
                }
            }
            _ => {}
        }
    }

    Some(vol_info)
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
