//! The microphones there are, and which is a Bluetooth headset's.

/// A microphone: its id (empty where the system has none) and name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Device {
    pub id: String,
    pub name: String,
}

/// Whether a microphone is a Bluetooth headset's, by its name. While a program records one, the
/// headset switches to its hands-free mode and everything else it plays sounds much worse, so
/// it's recorded only during calls (when the calling app has switched it already).
pub fn is_bluetooth(name: &str) -> bool {
    let n = name.to_lowercase();
    n.contains("hands-free")
        || n.contains("handsfree")
        || n.contains("bluetooth")
        || n.contains(" bt ")
        || n.starts_with("bt ")
        || n.contains("airpods")
}

#[cfg(target_os = "windows")]
pub fn microphones() -> Vec<Device> {
    crate::wasapi_capture::microphones()
}

#[cfg(target_os = "windows")]
pub fn default_microphone(communications: bool) -> Option<Device> {
    crate::wasapi_capture::default_microphone(communications)
}

#[cfg(not(target_os = "windows"))]
pub fn microphones() -> Vec<Device> {
    crate::cpal_capture::microphones()
}

#[cfg(not(target_os = "windows"))]
pub fn default_microphone(_communications: bool) -> Option<Device> {
    crate::cpal_capture::default_microphone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bluetooth_headsets_by_name() {
        assert!(is_bluetooth("Headset (WH-1000XM4 Hands-Free AG Audio)"));
        assert!(is_bluetooth("Microphone (AirPods Pro)"));
        assert!(is_bluetooth("Headset Microphone (Bluetooth)"));
        assert!(!is_bluetooth("Microphone (Realtek(R) Audio)"));
        assert!(!is_bluetooth("Headset Microphone (Jabra Evolve2 65)"));
        assert!(!is_bluetooth("Microphone (Brio 4K Stream Edition)"));
    }
}
