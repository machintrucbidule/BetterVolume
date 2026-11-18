//! Audio vocabulary shared by the engine and the rest of the application.
//!
//! Plain data with no Windows types: the audio engine (`bv-audio`) translates what Windows
//! reports into these types, so the rules built on them ([`crate::devices`],
//! [`crate::apps`]) can be tested without any audio device.

/// Direction of a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Flow {
    /// Playback device ("output").
    Output,
    /// Recording device ("input", microphone).
    Input,
}

impl Flow {
    /// Both flows, outputs first.
    pub const ALL: [Flow; 2] = [Flow::Output, Flow::Input];

    /// Position of this flow in a two-element array indexed by flow.
    pub fn index(self) -> usize {
        match self {
            Flow::Output => 0,
            Flow::Input => 1,
        }
    }
}

/// A Windows default-device role, in the application's terms.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Role {
    /// Media: the Windows "console" and "multimedia" roles together. The current device of
    /// a flow is its media default.
    Media,
    /// Calls: the Windows "communications" role.
    Calls,
}

impl Role {
    /// Both roles, media first.
    pub const ALL: [Role; 2] = [Role::Media, Role::Calls];

    /// Position of this role in a two-element array indexed by role.
    pub fn index(self) -> usize {
        match self {
            Role::Media => 0,
            Role::Calls => 1,
        }
    }
}

/// Physical type of a device as declared by its driver (FR-067). Drives the automatic icon.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FormFactor {
    /// Speakers.
    Speakers,
    /// Headphones.
    Headphones,
    /// Headset (headphones with a microphone).
    Headset,
    /// Telephone handset.
    Handset,
    /// Microphone.
    Microphone,
    /// Line-level connection (line in / line out).
    LineLevel,
    /// Digital S/PDIF connection.
    Spdif,
    /// Display with audio (monitor or TV over HDMI / DisplayPort).
    Display,
    /// Device reached over the network.
    Remote,
    /// Not declared, or a type the application does not distinguish.
    #[default]
    Unknown,
}

/// How a device is connected, when Windows tells it. Refines the automatic icon.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Bus {
    /// Bluetooth.
    Bluetooth,
    /// USB.
    Usb,
    /// Anything else (built-in, HDMI, virtual...), or unknown.
    #[default]
    Other,
}

/// Availability of a device (FR-043).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DeviceState {
    /// Present and enabled: can be used.
    #[default]
    Active,
    /// Disabled by the user in Windows.
    Disabled,
    /// Known but its jack or connection is unplugged (or the device is switched off).
    Unplugged,
}

impl DeviceState {
    /// Whether the device can be used right now.
    pub fn is_available(self) -> bool {
        self == DeviceState::Active
    }
}

/// A device as reported by Windows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Stable Windows identifier (FR-066). Survives unplug/replug and restarts.
    pub id: String,
    /// The name Windows shows for the device.
    pub windows_name: String,
    /// Declared physical type.
    pub form_factor: FormFactor,
    /// Connection.
    pub bus: Bus,
    /// Availability.
    pub state: DeviceState,
}

/// Maps a Windows endpoint enumerator name (the bus that exposes the device) to a [`Bus`].
pub fn bus_from_enumerator(name: &str) -> Bus {
    let name = name.trim().to_ascii_uppercase();
    if name.starts_with("BTH") {
        Bus::Bluetooth
    } else if name == "USB" {
        Bus::Usb
    } else {
        Bus::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexes_are_distinct_and_in_order() {
        assert_eq!(Flow::ALL.map(Flow::index), [0, 1]);
        assert_eq!(Role::ALL.map(Role::index), [0, 1]);
    }

    #[test]
    fn only_active_devices_are_available() {
        assert!(DeviceState::Active.is_available());
        assert!(!DeviceState::Disabled.is_available());
        assert!(!DeviceState::Unplugged.is_available());
    }

    #[test]
    fn enumerator_names_are_mapped() {
        assert_eq!(bus_from_enumerator("BTHENUM"), Bus::Bluetooth);
        assert_eq!(bus_from_enumerator("BTHHFENUM"), Bus::Bluetooth);
        assert_eq!(bus_from_enumerator("bthledevice"), Bus::Bluetooth);
        assert_eq!(bus_from_enumerator("USB"), Bus::Usb);
        assert_eq!(bus_from_enumerator(" usb "), Bus::Usb);
        assert_eq!(bus_from_enumerator("HDAUDIO"), Bus::Other);
        assert_eq!(bus_from_enumerator("USBSTOR"), Bus::Other);
        assert_eq!(bus_from_enumerator(""), Bus::Other);
    }
}
