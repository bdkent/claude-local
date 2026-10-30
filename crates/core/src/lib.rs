//! Platform logic for claude-local, kept free of any UI so it can be tested
//! headless.

pub mod claude;
pub mod config;
pub mod doctor;
pub mod env;
pub mod keepalive;
pub mod session;
pub mod terminal;

pub use config::Config;
pub use doctor::Check;
pub use env::ToolEnv;
pub use keepalive::{Keepalive, Report};
pub use session::{Dot, DotColor, Session, SessionKind};
pub use terminal::{Target, TerminalApp};
