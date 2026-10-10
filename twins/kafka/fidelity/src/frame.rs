//! Frames: one request and its response as they crossed the wire, kept in a
//! trace beside the history decoded from them, and the v2 record batches
//! inside produce requests and fetch responses.

use bytes::Bytes;
use kafka_protocol::records::{Record, RecordBatchDecoder};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// One request and its response, as they crossed the wire.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    /// Its position in the order the requests were sent, from 1.
    pub n: u64,
    /// The client connection it was sent on, from 1.
    pub conn: u64,
    /// The API's name.
    pub api: String,
    /// The API key.
    pub key: i16,
    /// The version.
    pub version: i16,
    /// The request as the broker got it: header and body, without the length.
    #[serde(with = "hex")]
    pub request: Vec<u8>,
    /// The broker's response: header and body, without the length; none
    /// when the broker sends none (a produce with `acks=0`).
    #[serde(default, skip_serializing_if = "Option::is_none", with = "opt_hex")]
    pub response: Option<Vec<u8>>,
    /// What the client got instead, when a fault changed the answer.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "opt_hex")]
    pub delivered: Option<Vec<u8>>,
    /// When the request was sent, in microseconds from the start.
    pub sent_us: u64,
    /// When its response came back, in microseconds from the start.
    pub answered_us: u64,
    /// Who sent it: `wire` for a client, `force` for a forced state.
    #[serde(default = "wire")]
    pub source: String,
}

impl Frame {
    /// The response the history is decoded from: the broker's, except for a
    /// fetch the recorder cut short, whose history is what the client got,
    /// as a twin writes a fetch under the consumer-lag fault.
    pub fn answer_for_history(&self) -> Option<&Vec<u8>> {
        if self.key == 1 { self.delivered.as_ref().or(self.response.as_ref()) } else { self.response.as_ref() }
    }
}

fn wire() -> String {
    "wire".into()
}

/// Lower-case hex.
pub fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Bytes from hex.
pub fn from_hex(text: &str) -> Result<Vec<u8>, String> {
    if text.len() % 2 != 0 {
        return Err("hex of odd length".into());
    }
    (0..text.len()).step_by(2).map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|e| e.to_string())).collect()
}

mod hex {
    use super::*;
    pub fn serialize<S: Serializer>(b: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&to_hex(b))
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let t = String::deserialize(d)?;
        from_hex(&t).map_err(serde::de::Error::custom)
    }
}

mod opt_hex {
    use super::*;
    pub fn serialize<S: Serializer>(b: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        match b {
            Some(b) => s.serialize_str(&to_hex(b)),
            None => s.serialize_none(),
        }
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        let t = Option::<String>::deserialize(d)?;
        t.map(|t| from_hex(&t).map_err(serde::de::Error::custom)).transpose()
    }
}

/// Reads frames written one per line.
pub fn parse_frames(text: &str) -> Result<Vec<Frame>, String> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .enumerate()
        .map(|(i, l)| serde_json::from_str(l).map_err(|e| format!("frame line {}: {e}", i + 1)))
        .collect()
}

/// Writes frames one per line.
pub fn frames_text(frames: &[Frame]) -> String {
    let mut s = String::new();
    for f in frames {
        s.push_str(&serde_json::to_string(f).unwrap_or_default());
        s.push('\n');
    }
    s
}

/// A v2 record batch, with what its header says and its records.
#[derive(Clone, Debug)]
pub struct Batch {
    /// The first offset.
    pub base: i64,
    /// The last offset, from the header: an empty batch still spans offsets.
    pub last: i64,
    /// The producer id, or -1.
    pub producer_id: i64,
    /// The producer's epoch.
    pub producer_epoch: i16,
    /// The first record's sequence number.
    pub base_sequence: i32,
    /// Whether it belongs to a transaction.
    pub transactional: bool,
    /// Whether it is a transaction marker.
    pub control: bool,
    /// The records.
    pub records: Vec<Record>,
    /// The batch as it was on the wire.
    pub raw: Bytes,
}

fn be_i64(b: &[u8], at: usize) -> i64 {
    i64::from_be_bytes(b[at..at + 8].try_into().unwrap())
}

fn be_i32(b: &[u8], at: usize) -> i32 {
    i32::from_be_bytes(b[at..at + 4].try_into().unwrap())
}

fn be_i16(b: &[u8], at: usize) -> i16 {
    i16::from_be_bytes(b[at..at + 2].try_into().unwrap())
}

/// The v2 batches in a records field, in order. A batch cut short at the
/// end, as a fetch may return, is left out; anything else that does not
/// decode is an error.
pub fn batches(bytes: &Bytes) -> Result<Vec<Batch>, String> {
    let mut out = vec![];
    let mut at = 0usize;
    let b = bytes.as_ref();
    while at + 61 <= b.len() {
        let length = be_i32(b, at + 8);
        if length < 49 {
            return Err(format!("a record batch of {length} bytes"));
        }
        let end = at + 12 + length as usize;
        if end > b.len() {
            break;
        }
        if b[at + 16] != 2 {
            return Err(format!("a record batch of magic {}", b[at + 16]));
        }
        let attributes = be_i16(b, at + 21);
        let base = be_i64(b, at);
        let mut slice = bytes.slice(at..end);
        let set = RecordBatchDecoder::decode(&mut slice).map_err(|e| e.to_string())?;
        out.push(Batch {
            base,
            last: base + be_i32(b, at + 23) as i64,
            producer_id: be_i64(b, at + 43),
            producer_epoch: be_i16(b, at + 51),
            base_sequence: be_i32(b, at + 53),
            transactional: attributes & (1 << 4) != 0,
            control: attributes & (1 << 5) != 0,
            records: set.records,
            raw: bytes.slice(at..end),
        });
        at = end;
    }
    Ok(out)
}
