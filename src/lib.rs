pub mod cli;
pub mod colors;
pub mod compare;
pub mod config;
pub mod ctx;
pub mod error;
pub mod fsops;
pub mod paths;
pub mod prompt;
pub mod switcher;
pub mod templates;
pub mod wizard;

pub use config::{AppConfig, Config, DefaultConfig};
pub use ctx::Ctx;
pub use error::{Error, Result};
pub use switcher::Switcher;
