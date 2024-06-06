use auxon_sdk::plugin_utils::serde::from_str;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub use crate::dbc::Dbc;
pub use crate::parser::{CanParser, ParsedCanFrame};
pub use convert::TimelineKey;
pub use send::Sender;

mod convert;
mod dbc;
mod parser;
mod send;

pub mod candump;

pub const PLUGIN_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct CommonConfig {
    /// Should the transmitting DBC node be used as the timeline identity and name? Defaults to true.
    #[serde(deserialize_with = "from_str", alias = "timeline_from_node")]
    pub timeline_from_node: Option<bool>,

    /// The default timeline name used when no DBC file is provided, or there are no transmitting
    /// nodes for a given CAN frame in the provided DBC.
    /// Defaults to 'canbus'.
    #[serde(alias = "default_timeline")]
    pub default_timeline: Option<String>,

    /// Use the DBC message name for event naming. Defaults to true.
    /// When no DBC file is provided, or there is no message definition, the CAN ID will be used.
    #[serde(deserialize_with = "from_str", alias = "event_from_message")]
    pub event_from_message: Option<bool>,

    /// DBC file to use when parsing the CAN frames.
    #[serde(deserialize_with = "from_str")]
    pub dbc: Option<PathBuf>,
}

pub trait HasCommonConfig {
    fn common_config(&self) -> &CommonConfig;
}
