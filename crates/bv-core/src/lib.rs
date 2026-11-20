//! BetterVolume pure logic.
//!
//! No Windows types and no I/O live here, so everything is unit-testable on its own:
//! translations ([`i18n`]), the settings schema ([`settings`]), the audio vocabulary
//! ([`audio`]), the device list model ([`devices`]), application rows ([`apps`]), volume
//! maths ([`volume`]), text helpers ([`text`]), the Home Assistant entities, topics and
//! payloads ([`ha`]), small byte encodings ([`encoding`]) and the release files and
//! version numbers ([`package`]) (see `docs/ARCHITECTURE.md`
//! §4).
#![forbid(unsafe_code)]

pub mod apps;
pub mod audio;
pub mod devices;
pub mod encoding;
pub mod ha;
pub mod i18n;
pub mod json;
pub mod package;
pub mod paths;
pub mod settings;
pub mod text;
pub mod update;
pub mod volume;
