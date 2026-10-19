pub mod error;
pub mod install;
pub mod locator;
pub mod mise;
pub mod registry;
pub mod project;
pub mod remote;
pub mod runner;
pub mod search;
pub mod tools;
pub mod types;

pub use error::CoreError;
pub use mise::Mise;
pub use locator::{default_candidates, locate_in, locate_mise};
pub use registry::parse_registry;
pub use project::{default_store_path, Project, ProjectStore};
pub use install::{ProcessStreamer, StreamEvent, StreamingRunner};
pub use remote::parse_remote_versions;
pub use runner::{CommandOutput, CommandRequest, CommandRunner, ProcessRunner};
pub use search::{SearchHit, SearchIndex};
pub use tools::{filter_rows, InstalledSet, Scope, ScopeFilter, ToolRow};
pub use types::{ConfigSource, InstalledTool, RemoteVersion, ToolEntry};

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
