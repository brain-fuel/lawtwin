//! The HTTP twin's core conformance kit: any core of the HTTP twin, in any
//! language, driven only over HTTP and the admin API
//! (docs/reference/http-core.md). ref:DEC-core-conformance-kit

pub mod core;
pub mod session;

/// Where the golden sessions are.
pub fn golden_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../shared/golden")
}

/// The contracts the kit checks a core with.
pub const CONTRACTS: &[&str] = &["petstore", "github"];
