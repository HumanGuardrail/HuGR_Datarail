//! `datarail-broker` — FROZEN API (frozen by the lead); WP4 implements the bodies + gates.
//! A minimal length-prefixed request/response protocol exposing a [`datarail_topic`] over a byte stream
//! (TCP/loopback), with durable group offsets via [`datarail_offsets`].
#![forbid(unsafe_code)]

/// A client request over the wire (WP4: define the on-wire encoding; this enum is the frozen shape).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Append a record.
    Produce {
        /// Routing/order key.
        key: Vec<u8>,
        /// Payload bytes.
        payload: Vec<u8>,
    },
    /// Dispatch the next record for a group member.
    Poll {
        /// Consumer group name.
        group: String,
    },
    /// Durably commit a group's offset.
    Commit {
        /// Consumer group name.
        group: String,
        /// Offset to commit.
        offset: u64,
    },
}

/// A server response (WP4: frozen shape).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Response {
    /// Produce accepted at this offset.
    Produced(u64),
    /// A dispatched record, or `None` at the tail.
    Record(Option<(u64, Vec<u8>, Vec<u8>)>),
    /// Commit acknowledged.
    Committed,
    /// An error string.
    Err(String),
}

/// Encode a [`Request`] to wire bytes. WP4: implement (length-prefixed, CRC-tailed like the rest of the codebase).
#[must_use]
pub fn encode_request(_req: &Request) -> Vec<u8> {
    Vec::new() // WP4
}

/// Decode a [`Request`] from a complete frame. WP4: implement.
#[must_use]
pub fn decode_request(_frame: &[u8]) -> Option<Request> {
    None // WP4
}
