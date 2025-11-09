use std::collections::HashMap;

use crate::utils::VolumeInfo;

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
            "Audio/Source" => DeviceKind::Sink,
            "Audio/Device" => DeviceKind::Device,
            _ => DeviceKind::Unknown,
        }
    }
}

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

#[derive(Debug, Default)]
pub struct State {
    pub devices: HashMap<u32, Entry>,
    pub nodes: HashMap<u32, Entry>,
}

#[derive(Debug)]
pub enum ActionType {
    EntryAdd(Entry),
    EntryRemove(u32),
    VolumeChange(u32, VolumeInfo),
    Shutdown,
}
