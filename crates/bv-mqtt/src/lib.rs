//! BetterVolume Home Assistant link: the MQTT connection (functional specification,
//! section 8).
//!
//! [`codec`] reads and writes MQTT 3.1.1 packets; [`client`] keeps one connection to the
//! broker alive (optional encryption through Windows, reconnection with increasing delay,
//! keep-alive, last will) and reports what happens through a channel. What is published
//! (discovery, states) and how commands are understood is built by `bv_core::ha`, where it
//! is unit-tested; the application wires both to the audio engine. The client is created
//! only while the integration is on, so it costs nothing otherwise (FR-118).
//!
//! [`broker`] (feature `test-broker`, and in tests) is a small broker for automated tests
//! and the footprint measurement; it is never part of the application.
#![forbid(unsafe_code)]

#[cfg(any(test, feature = "test-broker"))]
pub mod broker;
pub mod client;
pub mod codec;

pub use client::{Client, Event, Fingerprint, Message, Notify, Options, Problem, Refusal, Tls};
pub use codec::Will;

#[cfg(test)]
mod tests;
