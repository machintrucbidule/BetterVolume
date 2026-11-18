//! MQTT 3.1.1 packets: encoding and incremental decoding (OASIS standard, sections 2 and 3).
//!
//! Only what BetterVolume and its test broker use: CONNECT/CONNACK, PUBLISH (QoS 0 and 1)
//! and PUBACK, SUBSCRIBE/SUBACK, PINGREQ/PINGRESP and DISCONNECT. Anything else, or a
//! malformed packet, is a [`DecodeError`] and the connection is dropped.

/// Largest packet accepted (a discovery configuration is a few kilobytes).
pub const MAX_PACKET: usize = 1 << 20;

/// The last will of a connection (published by the broker if it is lost uncleanly).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Will {
    /// Topic.
    pub topic: String,
    /// Payload.
    pub payload: Vec<u8>,
    /// Retained.
    pub retain: bool,
}

/// The CONNECT packet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Connect {
    /// Client identifier.
    pub client_id: String,
    /// Keep-alive interval in seconds.
    pub keep_alive: u16,
    /// Start a clean session.
    pub clean_session: bool,
    /// Last will.
    pub will: Option<Will>,
    /// User name.
    pub username: Option<String>,
    /// Password.
    pub password: Option<Vec<u8>>,
}

/// A PUBLISH packet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Publish {
    /// Topic.
    pub topic: String,
    /// Payload.
    pub payload: Vec<u8>,
    /// Quality of service (0 or 1).
    pub qos: u8,
    /// Retained (received: delivered from the broker's store).
    pub retain: bool,
    /// Packet identifier (QoS 1 only).
    pub id: u16,
}

/// One MQTT packet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Packet {
    /// Connection request.
    Connect(Connect),
    /// Connection answer.
    ConnAck {
        /// The broker kept a session.
        session_present: bool,
        /// 0 = accepted; 1 to 5 = refused (see the standard, 3.2.2.3).
        code: u8,
    },
    /// A message.
    Publish(Publish),
    /// Acknowledgement of a QoS 1 message.
    PubAck(u16),
    /// Subscription request: packet identifier, topic filters with their QoS.
    Subscribe(u16, Vec<(String, u8)>),
    /// Subscription answer: packet identifier, one code per filter (0x80 = refused).
    SubAck(u16, Vec<u8>),
    /// Keep-alive request.
    PingReq,
    /// Keep-alive answer.
    PingResp,
    /// Clean disconnection (the will is not published).
    Disconnect,
}

/// A packet that cannot be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The remaining length is malformed or too large.
    Length,
    /// A field is malformed (bad string, bad flags, unknown type).
    Malformed,
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecodeError::Length => f.write_str("bad packet length"),
            DecodeError::Malformed => f.write_str("malformed packet"),
        }
    }
}

impl std::error::Error for DecodeError {}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    // Every field this crate writes is far below 64 KiB (topics, names, small payloads).
    put_u16(out, u16::try_from(bytes.len()).unwrap_or(u16::MAX));
    out.extend_from_slice(&bytes[..bytes.len().min(usize::from(u16::MAX))]);
}

fn frame(kind: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 5);
    out.push(kind);
    let mut len = body.len();
    loop {
        let mut byte = (len % 128) as u8;
        len /= 128;
        if len > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if len == 0 {
            break;
        }
    }
    out.extend_from_slice(body);
    out
}

impl Packet {
    /// The bytes of the packet.
    pub fn encode(&self) -> Vec<u8> {
        let mut body = Vec::new();
        let kind = match self {
            Packet::Connect(c) => {
                put_bytes(&mut body, b"MQTT");
                body.push(4);
                let mut flags = 0_u8;
                if c.clean_session {
                    flags |= 0x02;
                }
                if let Some(will) = &c.will {
                    flags |= 0x04;
                    if will.retain {
                        flags |= 0x20;
                    }
                }
                if c.password.is_some() {
                    flags |= 0x40;
                }
                if c.username.is_some() {
                    flags |= 0x80;
                }
                body.push(flags);
                put_u16(&mut body, c.keep_alive);
                put_bytes(&mut body, c.client_id.as_bytes());
                if let Some(will) = &c.will {
                    put_bytes(&mut body, will.topic.as_bytes());
                    put_bytes(&mut body, &will.payload);
                }
                if let Some(user) = &c.username {
                    put_bytes(&mut body, user.as_bytes());
                }
                if let Some(password) = &c.password {
                    put_bytes(&mut body, password);
                }
                0x10
            }
            Packet::ConnAck { session_present, code } => {
                body.push(u8::from(*session_present));
                body.push(*code);
                0x20
            }
            Packet::Publish(p) => {
                put_bytes(&mut body, p.topic.as_bytes());
                if p.qos > 0 {
                    put_u16(&mut body, p.id);
                }
                body.extend_from_slice(&p.payload);
                0x30 | ((p.qos.min(1)) << 1) | u8::from(p.retain)
            }
            Packet::PubAck(id) => {
                put_u16(&mut body, *id);
                0x40
            }
            Packet::Subscribe(id, filters) => {
                put_u16(&mut body, *id);
                for (filter, qos) in filters {
                    put_bytes(&mut body, filter.as_bytes());
                    body.push(*qos);
                }
                0x82
            }
            Packet::SubAck(id, codes) => {
                put_u16(&mut body, *id);
                body.extend_from_slice(codes);
                0x90
            }
            Packet::PingReq => 0xC0,
            Packet::PingResp => 0xD0,
            Packet::Disconnect => 0xE0,
        };
        frame(kind, &body)
    }

    /// The first complete packet at the start of `input` and its length in bytes;
    /// `Ok(None)` if more bytes are needed.
    pub fn decode(input: &[u8]) -> Result<Option<(Packet, usize)>, DecodeError> {
        let Some(&first) = input.first() else { return Ok(None) };
        let mut len = 0_usize;
        let mut shift = 0;
        let mut header = 1;
        loop {
            let Some(&byte) = input.get(header) else { return Ok(None) };
            header += 1;
            len |= usize::from(byte & 0x7F) << shift;
            if byte & 0x80 == 0 {
                break;
            }
            shift += 7;
            if header > 4 {
                return Err(DecodeError::Length);
            }
        }
        if len > MAX_PACKET {
            return Err(DecodeError::Length);
        }
        let Some(body) = input.get(header..header + len) else { return Ok(None) };
        let packet = parse(first, body).ok_or(DecodeError::Malformed)?;
        Ok(Some((packet, header + len)))
    }
}

/// Reads fields from a packet body.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn u8(&mut self) -> Option<u8> {
        let (&b, rest) = self.0.split_first()?;
        self.0 = rest;
        Some(b)
    }

    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_be_bytes([self.u8()?, self.u8()?]))
    }

    fn bytes(&mut self) -> Option<&'a [u8]> {
        let len = usize::from(self.u16()?);
        let (field, rest) = (self.0.get(..len)?, self.0.get(len..)?);
        self.0 = rest;
        Some(field)
    }

    fn text(&mut self) -> Option<String> {
        let text = std::str::from_utf8(self.bytes()?).ok()?;
        // MQTT strings must not hold U+0000 (standard 1.5.3).
        (!text.contains('\0')).then(|| text.to_owned())
    }

    fn done(&self) -> bool {
        self.0.is_empty()
    }
}

fn parse(first: u8, body: &[u8]) -> Option<Packet> {
    let mut r = Reader(body);
    let flags = first & 0x0F;
    let packet = match first >> 4 {
        1 if flags == 0 => {
            if r.bytes()? != b"MQTT" || r.u8()? != 4 {
                return None;
            }
            let connect_flags = r.u8()?;
            if connect_flags & 0x01 != 0 {
                return None;
            }
            let keep_alive = r.u16()?;
            let client_id = r.text()?;
            let will = if connect_flags & 0x04 != 0 {
                let topic = r.text()?;
                let payload = r.bytes()?.to_vec();
                Some(Will { topic, payload, retain: connect_flags & 0x20 != 0 })
            } else {
                None
            };
            let username = if connect_flags & 0x80 != 0 { Some(r.text()?) } else { None };
            let password = if connect_flags & 0x40 != 0 { Some(r.bytes()?.to_vec()) } else { None };
            Packet::Connect(Connect {
                client_id,
                keep_alive,
                clean_session: connect_flags & 0x02 != 0,
                will,
                username,
                password,
            })
        }
        2 if flags == 0 => Packet::ConnAck { session_present: r.u8()? & 1 == 1, code: r.u8()? },
        3 => {
            let qos = (flags >> 1) & 0x03;
            if qos > 1 {
                return None;
            }
            let topic = r.text()?;
            let id = if qos > 0 { r.u16()? } else { 0 };
            let payload = std::mem::take(&mut r.0).to_vec();
            Packet::Publish(Publish { topic, payload, qos, retain: flags & 1 == 1, id })
        }
        4 if flags == 0 => Packet::PubAck(r.u16()?),
        8 if flags == 2 => {
            let id = r.u16()?;
            let mut filters = Vec::new();
            while !r.done() {
                let filter = r.text()?;
                filters.push((filter, r.u8()? & 0x03));
            }
            if filters.is_empty() {
                return None;
            }
            Packet::Subscribe(id, filters)
        }
        9 if flags == 0 => {
            let id = r.u16()?;
            Packet::SubAck(id, std::mem::take(&mut r.0).to_vec())
        }
        12 if flags == 0 => Packet::PingReq,
        13 if flags == 0 => Packet::PingResp,
        14 if flags == 0 => Packet::Disconnect,
        _ => return None,
    };
    r.done().then_some(packet)
}

/// Whether `topic` matches the subscription `filter` (wildcards `+` and `#`).
pub fn matches(filter: &str, topic: &str) -> bool {
    let mut levels = topic.split('/');
    for part in filter.split('/') {
        match part {
            "#" => return true,
            "+" => {
                if levels.next().is_none() {
                    return false;
                }
            }
            exact => {
                if levels.next() != Some(exact) {
                    return false;
                }
            }
        }
    }
    levels.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(packet: Packet) {
        let bytes = packet.encode();
        assert_eq!(Packet::decode(&bytes), Ok(Some((packet.clone(), bytes.len()))), "{packet:?}");
        // Every shorter prefix asks for more bytes.
        for cut in 0..bytes.len() {
            assert_eq!(Packet::decode(&bytes[..cut]), Ok(None), "{packet:?} cut at {cut}");
        }
    }

    #[test]
    fn every_packet_reads_back() {
        round_trip(Packet::Connect(Connect {
            client_id: "bettervolume-abc".into(),
            keep_alive: 30,
            clean_session: true,
            will: Some(Will {
                topic: "a/availability".into(),
                payload: b"offline".to_vec(),
                retain: true,
            }),
            username: Some("user é".into()),
            password: Some(b"secret".to_vec()),
        }));
        round_trip(Packet::Connect(Connect {
            client_id: String::new(),
            keep_alive: 0,
            clean_session: false,
            will: None,
            username: None,
            password: None,
        }));
        round_trip(Packet::ConnAck { session_present: false, code: 4 });
        round_trip(Packet::Publish(Publish {
            topic: "t/x".into(),
            payload: vec![1, 2, 3],
            qos: 0,
            retain: true,
            id: 0,
        }));
        round_trip(Packet::Publish(Publish {
            topic: "t".into(),
            payload: vec![b'x'; 300],
            qos: 1,
            retain: false,
            id: 7,
        }));
        round_trip(Packet::PubAck(7));
        round_trip(Packet::Subscribe(1, vec![("a/+/set".into(), 0), ("b/#".into(), 1)]));
        round_trip(Packet::SubAck(1, vec![0, 0x80]));
        round_trip(Packet::PingReq);
        round_trip(Packet::PingResp);
        round_trip(Packet::Disconnect);
    }

    #[test]
    fn the_standard_encoding_is_produced() {
        assert_eq!(Packet::PingReq.encode(), [0xC0, 0x00]);
        assert_eq!(Packet::Disconnect.encode(), [0xE0, 0x00]);
        let connect = Packet::Connect(Connect {
            client_id: "c".into(),
            keep_alive: 60,
            clean_session: true,
            will: None,
            username: None,
            password: None,
        });
        assert_eq!(
            connect.encode(),
            [0x10, 13, 0, 4, b'M', b'Q', b'T', b'T', 4, 0x02, 0, 60, 0, 1, b'c']
        );
        // Remaining length of 321 bytes: two length bytes (0xC1 0x02).
        let big = Packet::Publish(Publish {
            topic: "t".into(),
            payload: vec![0; 318],
            qos: 0,
            retain: false,
            id: 0,
        });
        assert_eq!(&big.encode()[..3], [0x30, 0xC1, 0x02]);
    }

    #[test]
    fn two_packets_in_a_row_are_read_one_by_one() {
        let mut bytes = Packet::PingResp.encode();
        bytes.extend(Packet::PubAck(3).encode());
        let (first, used) = Packet::decode(&bytes).unwrap().unwrap();
        assert_eq!((first, used), (Packet::PingResp, 2));
        assert_eq!(Packet::decode(&bytes[used..]).unwrap().unwrap().0, Packet::PubAck(3));
    }

    #[test]
    fn malformed_packets_are_refused() {
        for bad in [
            &[0xC0, 0x01, 0x00][..],               // ping with a body
            &[0xF0, 0x00],                         // reserved type
            &[0x36, 0x03, 0x00, 0x01, b't'],       // QoS 3
            &[0x80, 0x02, 0x00, 0x01],             // subscribe without filters (bad flags)
            &[0x82, 0x02, 0x00, 0x01],             // subscribe without filters
            &[0x30, 0x04, 0x00, 0x05, b'a', b'b'], // topic longer than the packet
            &[0x30, 0x03, 0x00, 0x01, 0x00],       // NUL in the topic
            &[0x30, 0x04, 0x00, 0x02, 0xC3, 0x28], // invalid UTF-8
        ] {
            assert_eq!(Packet::decode(bad), Err(DecodeError::Malformed), "{bad:?}");
        }
        assert_eq!(Packet::decode(&[0x30, 0xFF, 0xFF, 0xFF, 0xFF, 0x01]), Err(DecodeError::Length));
        assert_eq!(Packet::decode(&[0x30, 0xFF, 0xFF, 0xFF, 0x7F]), Err(DecodeError::Length));
    }

    #[test]
    fn topic_filters_match_like_the_standard() {
        for (filter, topic, expected) in [
            ("a/b", "a/b", true),
            ("a/b", "a/c", false),
            ("a/+", "a/b", true),
            ("a/+", "a/b/c", false),
            ("a/#", "a", true),
            ("a/#", "a/b/c", true),
            ("#", "anything/at/all", true),
            ("+/+/set", "x/y/set", true),
            ("a/b/c", "a/b", false),
        ] {
            assert_eq!(matches(filter, topic), expected, "{filter} vs {topic}");
        }
    }
}
