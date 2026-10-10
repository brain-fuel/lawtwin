//! The core conformance kit of the Kafka twin. Any core, in any language,
//! must pass it, driven only over the Kafka protocol and the admin API:
//! real clients (librdkafka, kafkajs, franz-go, the Java client), the laws
//! checked over each history by the kit itself, faults, forced states and
//! the clock, same-seed determinism, and the golden session every core must
//! answer byte for byte. `make core-conformance CORE=<command or image:...>`
//! runs it. ref:DEC-core-conformance-kit

pub mod core;
pub mod golden;
pub mod session;
