//! The registry of compiled contracts: every imported contract under
//! `contracts/` with a generated Rust test project, as `build.rs` brings it
//! in. A core serves the one it is started with. ref:DEC-http-contract-registry

use crate::model::Contract;

include!(concat!(env!("OUT_DIR"), "/contracts.rs"));

/// The compiled contract with this name (`petstore`) or package (`lawtwin.petstore`).
pub fn find(name: &str) -> Option<Contract> {
    registry().into_iter().find(|c| c.name == name || c.package == name)
}
