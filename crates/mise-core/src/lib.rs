pub mod error;
pub mod locator;
pub mod registry;
pub mod runner;
pub mod search;
pub mod types;

pub use error::CoreError;
pub use locator::{default_candidates, locate_in, locate_mise};
pub use registry::parse_registry;
pub use runner::{CommandOutput, CommandRequest, CommandRunner, ProcessRunner};
pub use search::{SearchHit, SearchIndex};
pub use types::ToolEntry;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
