//! The reference model a contract's generated code gives the twin, behind
//! one interface for every compiled contract.
//!
//! The importer emits a `Model` adapter calling each contract's public
//! generated handler through its failure-returning embedding interface,
//! and gives a raised `ApiError` back as its constructor's name.
//! ref:DEC-embed-generated-model

use crate::V;

/// Why an operation of the model gave no result.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// The contract refused the call with this `ApiError` constructor.
    Failure(String),
    /// The generated code failed; this is a defect, never a refusal.
    Defect(String),
}

/// A contract's reference model, with its state.
pub trait Model: Send {
    /// Runs an operation with its arguments, in the operation's order, and
    /// keeps the next state.
    fn call(&mut self, op: &str, args: Vec<V>) -> Result<V, Outcome>;
    /// The state, as a value of the contract's `State`.
    fn state(&self) -> V;
    /// A copy, for forcing a state on it first.
    fn boxed(&self) -> Box<dyn Model>;
}

/// A contract compiled into the core.
#[derive(Clone, Copy)]
pub struct Contract {
    /// Its directory's name under `contracts/`, such as `petstore`.
    pub name: &'static str,
    /// Its package, such as `lawtwin.petstore`.
    pub package: &'static str,
    pub version: &'static str,
    /// Its `lawtwin-openapi.json`.
    pub wire_map: &'static str,
    /// Its `lawtwin.json`.
    pub twin_file: &'static str,
    /// Its model in the start state.
    pub start: fn() -> Box<dyn Model>,
}

/// The text of a panic's payload.
pub fn panic_text(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else {
        "a panic in the generated model".into()
    }
}
