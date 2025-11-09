use pipewire::{
    spa::pod::{Pod, Value, ValueArray, deserialize::PodDeserializer},
    spa::utils::dict::DictRef,
    thread_loop::ThreadLoopRc,
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

#[derive(Debug, Clone, Default)]
pub struct VolumeInfo {
    volume: Option<f32>,
    mute: Option<bool>,
    channel_volumes: Vec<f32>,
}

impl VolumeInfo {
    pub fn from_pod(param: &Pod) -> Option<VolumeInfo> {
        // TODO: try_from ?
        let obj = param.as_object().ok()?;
        let mut vol_info = VolumeInfo::default();

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

    pub fn format_display(&self) -> Option<String> {
        let mut parts = Vec::new();

        if let Some(vol) = self.volume {
            parts.push(format!("Volume: {:.0}%", vol * 100.0));
        }

        if let Some(m) = self.mute {
            parts.push(format!("Mute: {}", if m { "ON" } else { "OFF" }));
        }

        if !self.channel_volumes.is_empty() {
            let channels = self
                .channel_volumes
                .iter()
                .enumerate()
                .map(|(i, &v)| format!("Ch{}: {:.0}%", i + 1, v * 100.0))
                .collect::<Vec<_>>()
                .join(", ");
            parts.push(format!("Channels: [{channels}]"));
        }

        if parts.is_empty() {
            Some("Property changed".to_string())
        } else {
            Some(parts.join(" | "))
        }
    }
}
