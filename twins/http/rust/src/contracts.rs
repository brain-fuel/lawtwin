//! Explicitly linked generated contract libraries. Importing a new contract
//! emits its adapter; register its Cargo dependency and module here.
//! ref:DEC-http-contract-registry
use crate::model::Contract;

#[allow(warnings, clippy::all)]
#[path = "../../../../contracts/github/lawtwin-embed.rs"]
pub mod github;
#[allow(warnings, clippy::all)]
#[path = "../../../../contracts/petstore/lawtwin-embed.rs"]
pub mod petstore;

pub fn registry() -> Vec<Contract> {
    vec![
        Contract {
            name: "github",
            package: github::PACKAGE,
            version: github::VERSION,
            wire_map: github::WIRE_MAP,
            twin_file: github::TWIN_FILE,
            start: github::start,
        },
        Contract {
            name: "petstore",
            package: petstore::PACKAGE,
            version: petstore::VERSION,
            wire_map: petstore::WIRE_MAP,
            twin_file: petstore::TWIN_FILE,
            start: petstore::start,
        },
    ]
}

/// The compiled contract with this name (`petstore`) or package (`lawtwin.petstore`).
pub fn find(name: &str) -> Option<Contract> {
    registry()
        .into_iter()
        .find(|c| c.name == name || c.package == name)
}
