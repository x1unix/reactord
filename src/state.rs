use std::collections::HashMap;

#[derive(Debug, Clone)]
pub enum DeviceKind {
    /// Unknown is fallback value.
    Unknown,

    /// Device is generic device that can be either source, sink or both.
    Device,

    /// Sink is output device (e.g. headphones).
    Sink,

    /// Source is input device (e.g. microphone).
    Source,
}

impl From<&str> for DeviceKind {
    fn from(value: &str) -> Self {
        match value {
            "Audio/Sink" => DeviceKind::Sink,
            "Audio/Source" => DeviceKind::Source,
            "Audio/Device" => DeviceKind::Device,
            _ => DeviceKind::Unknown,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct VolumeInfo {
    pub volume: Option<f32>,
    pub mute: Option<bool>,
    pub channel_volumes: Vec<f32>,
}

#[allow(dead_code)]
impl VolumeInfo {
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

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct Entry {
    pub id: u32,
    pub is_node: bool,
    pub device_id: Option<u32>,
    pub name: Option<String>,
    pub label: Option<String>,
    pub description: Option<String>,
    pub kind: DeviceKind,
    pub volume: Option<VolumeInfo>,
}

impl Entry {
    pub fn get_label(&self) -> &str {
        self.label
            .as_ref()
            .or(self.description.as_ref())
            .or(self.name.as_ref())
            .map(|v| v.as_str())
            .unwrap_or_else(|| "<unnamed>")
    }
}

#[allow(dead_code)]
#[derive(Debug, Default)]
pub struct State {
    pub notification_ids: HashMap<u32, u32>,
    pub devices: HashMap<u32, Entry>,
    pub nodes: HashMap<u32, Entry>,
}

#[allow(dead_code)]
#[derive(Debug)]
pub enum ActionType {
    EntryAdd(u32, Entry),
    EntryRemove(u32),
    VolumeChange(u32, VolumeInfo),
    Shutdown,
}
