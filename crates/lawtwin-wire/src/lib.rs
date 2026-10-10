//! The wire side of an imported OpenAPI contract.
//!
//! `lawtwin import openapi` writes `lawtwin-openapi.json` beside each
//! contract package: how the package's operations, types and failures look
//! on the wire. This crate reads it, and gives everything that is not a
//! twin's core the same view of it:
//!
//! - [`ty`]: the LawSpec type expressions the wire map names;
//! - [`map`]: the wire map itself;
//! - [`value`]: values of the contract's types, to and from JSON;
//! - [`call`]: requests as calls of the contract's operations, and back;
//! - [`schema`]: documented failure bodies, built and checked;
//! - [`history`] and [`laws`]: the history format, and the contract's
//!   laws checked over a history or a state;
//! - [`traffic`]: seeded values and requests, valid and invalid;
//! - [`http`]: a small HTTP/1.1 client that can also misbehave.
//!
//! Every twin core reads the same file the same way, whatever its language
//! (docs/reference/http-core.md); this crate is the Rust reading of it.

pub mod call;
pub mod history;
pub mod http;
pub mod laws;
pub mod map;
pub mod rng;
pub mod schema;
pub mod traffic;
pub mod ty;
pub mod value;

pub use map::WireMap;
pub use ty::Ty;
pub use value::V;
