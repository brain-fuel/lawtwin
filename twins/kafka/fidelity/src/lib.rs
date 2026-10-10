//! Fidelity of the Kafka twin: how far a twin can be trusted to behave like
//! real Kafka (ref:DEC-fidelity).
//!
//! - `record` is a Kafka protocol proxy that records a real broker's
//!   traffic as a trace, decodes it into the contract's history and checks
//!   it against the contract as it goes.
//! - `replay` sends a trace's requests through a twin core and reports where
//!   its answers part from the recorded ones.
//! - `conform` runs the core conformance kit's client scenarios and law
//!   checks against real Kafka through the recorder, and writes a
//!   conformance report.
//! - `peer` holds the simulated peers: a producer feeding topics an
//!   application consumes, and a consumer checking what it produces.

pub mod cli;
pub mod compare;
pub mod conform;
pub mod decode;
pub mod frame;
pub mod lawtype;
pub mod peer;
pub mod record;
pub mod replay;
