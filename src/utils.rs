use pipewire::{spa::utils::dict::DictRef, thread_loop::ThreadLoopRc};

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
