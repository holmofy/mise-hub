pub mod error;
pub mod install;
pub mod locator;
pub mod manage;
pub mod mise;
pub mod outdated;
pub mod project;
pub mod registry;
pub mod remote;
pub mod runner;
pub mod search;
pub mod settings;
pub mod tools;
pub mod types;

pub use error::CoreError;
pub use install::{ProcessStreamer, StreamEvent, StreamingRunner};
pub use locator::{default_candidates, locate_in, locate_mise};
pub use manage::{UsePreview, parse_use_dry_run};
pub use mise::Mise;
pub use outdated::parse_outdated;
pub use project::{Project, ProjectStore, default_store_path};
pub use registry::parse_registry;
pub use remote::parse_remote_versions;
pub use runner::{CommandOutput, CommandRequest, CommandRunner, ProcessRunner};
pub use search::{SearchHit, SearchIndex};
pub use settings::{AppSettings, default_settings_path};
pub use tools::{InstalledSet, Scope, ScopeFilter, ToolRow, filter_rows};
pub use types::{ConfigSource, InstalledTool, RemoteVersion, ToolEntry};

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
