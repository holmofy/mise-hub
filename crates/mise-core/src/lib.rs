pub mod error;
pub mod locator;
pub mod runner;

pub use error::CoreError;
pub use locator::{default_candidates, locate_in, locate_mise};
pub use runner::{CommandOutput, CommandRequest, CommandRunner, ProcessRunner};

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
