use anyhow::anyhow;
use auxon_sdk::plugin_utils::serde::from_str;
use auxon_sdk::{init_tracing, plugin_utils::ingest::Config};
use futures_util::StreamExt;
use modality_can::{CanParser, Dbc, HasCommonConfig, Sender, PLUGIN_VERSION};
use serde::{Deserialize, Serialize};
use socketcan::{
    nl::{CanBitTiming, CanCtrlMode, CanCtrlModes},
    tokio::CanFdSocket,
    CanInterface, SetCanParams, SocketOptions,
};
use std::str::FromStr;
use tokio_util::{sync::CancellationToken, task::TaskTracker};
use tracing::{debug, info};

/// Collect CAN data from a SocketCAN interface.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct CollectorConfig {
    /// The SocketCAN interface to use. Defaults to 'can0'.
    interface: Option<String>,

    /// List of CAN filters to apply.
    filters: Option<Vec<CanFilter>>,

    /// Enable receive hardware timestamps.
    /// Defaults to true.
    #[serde(deserialize_with = "from_str", alias = "hw_timestamps")]
    hw_timestamps: Option<bool>,

    /// Brings the interface up by settings its “up” flag enabled via netlink.
    /// Defaults to false.
    /// This is a privileged operation that requires the `CAP_NET_ADMIN` capability.
    #[serde(deserialize_with = "from_str", alias = "bring_up")]
    bring_up: Option<bool>,

    /// CAN bitrate.
    /// Defaults to unchanged.
    /// This is a privileged operation that requires the `CAP_NET_ADMIN` capability.
    #[serde(deserialize_with = "from_str")]
    bitrate: Option<u32>,

    /// CAN FD data bitrate.
    /// Defaults to unchanged.
    /// This is a privileged operation that requires the `CAP_NET_ADMIN` capability.
    #[serde(deserialize_with = "from_str", alias = "data_bitrate")]
    data_bitrate: Option<u32>,

    /// Set the automatic restart time (in milliseconds).
    /// Zero means auto-restart is disabled.
    /// Defaults to unchanged.
    /// This is a privileged operation that requires the `CAP_NET_ADMIN` capability.
    #[serde(deserialize_with = "from_str", alias = "restart_ms")]
    restart_ms: Option<u32>,

    /// The CAN bus termination resistance.
    /// Defaults to unchanged.
    /// This is a privileged operation that requires the `CAP_NET_ADMIN` capability.
    #[serde(deserialize_with = "from_str")]
    termination: Option<u16>,

    /// Set the listen-only control mode bit.
    /// Defaults to false.
    /// This is a privileged operation that requires the `CAP_NET_ADMIN` capability.
    #[serde(deserialize_with = "from_str", alias = "listen_only")]
    listen_only: Option<bool>,

    /// Set the CAN FD control mode bit.
    /// Defaults to false.
    /// This is a privileged operation that requires the `CAP_NET_ADMIN` capability.
    #[serde(deserialize_with = "from_str")]
    fd: Option<bool>,

    #[serde(flatten)]
    common: modality_can::CommonConfig,
}

impl HasCommonConfig for CollectorConfig {
    fn common_config(&self) -> &modality_can::CommonConfig {
        &self.common
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    init_tracing!(tracing_subscriber::EnvFilter::new(format!(
        "{}={},modality_socketcan={}",
        env!("CARGO_PKG_NAME").replace('-', "_"),
        tracing::Level::INFO,
        tracing::Level::INFO,
    )));

    let config = Config::<CollectorConfig>::load_custom("MODALITY_CAN_", |env_key, env_val| {
        if env_key == "FILTERS" {
            let filter_strs: Vec<&str> = env_val.split(',').collect();
            let mut filters = Vec::with_capacity(filter_strs.len());
            for f_str in filter_strs.iter() {
                let f = match CanFilter::from_str(f_str) {
                    Ok(f) => f,
                    Err(e) => {
                        let e: Box<dyn std::error::Error + Send + Sync> = e.into();
                        return Err(e);
                    }
                };
                let f_toml = match toml::Value::try_from(f) {
                    Ok(f) => f,
                    Err(e) => {
                        let e: Box<dyn std::error::Error + Send + Sync> =
                            format!("Failed to convert CAN filter into toml. {e}").into();
                        return Err(e);
                    }
                };
                filters.push(f_toml);
            }
            Ok(Some(("filters".to_owned(), toml::Value::Array(filters))))
        } else {
            Ok(None)
        }
    })?;

    let dbc = config
        .plugin
        .common
        .dbc
        .as_ref()
        .map(Dbc::from_file)
        .transpose()?;

    let mut parser = CanParser::new(&config.plugin.common, dbc.as_ref().map(|dbc| &dbc.inner))?;

    let iface = config.plugin.interface.as_deref().unwrap_or("can0");
    info!(interface = iface, "Opening CAN interface");

    let can_iface = CanInterface::open(iface)
        .map_err(|e| anyhow!("Failed to open CAN interface '{}'. {}", iface, e))?;

    // Configure the interface if asked to do so
    let uses_params = config.plugin.bitrate.is_some()
        || config.plugin.data_bitrate.is_some()
        || config.plugin.restart_ms.is_some()
        || config.plugin.termination.is_some()
        || config.plugin.listen_only.is_some()
        || config.plugin.fd.is_some();
    if uses_params {
        let mut params = SetCanParams::default();

        // Set/clear control mode bits
        if config.plugin.listen_only.is_some() || config.plugin.fd.is_some() {
            let mut ctrl_modes = CanCtrlModes::default();
            if let Some(on) = config.plugin.listen_only {
                ctrl_modes.add(CanCtrlMode::ListenOnly, on);
            }
            if let Some(on) = config.plugin.fd {
                ctrl_modes.add(CanCtrlMode::Fd, on);
            }
            params.ctrl_mode = Some(ctrl_modes);
        }

        if let Some(bitrate) = config.plugin.bitrate {
            params.bit_timing = Some(CanBitTiming {
                bitrate,
                ..Default::default()
            });
        }
        if let Some(bitrate) = config.plugin.data_bitrate {
            params.data_bit_timing = Some(CanBitTiming {
                bitrate,
                ..Default::default()
            });
        }
        if let Some(restart_ms) = config.plugin.restart_ms {
            params.restart_ms = Some(restart_ms);
        }
        if let Some(termination) = config.plugin.termination {
            params.termination = Some(termination);
        }

        can_iface.set_can_params(&params).map_err(|e| {
            anyhow!(
                "Failed to set CAN parameters on interface '{}'. {}",
                iface,
                e
            )
        })?;
    }

    if config.plugin.bring_up.unwrap_or(false) {
        can_iface
            .bring_up()
            .map_err(|e| anyhow!("Failed to bring up CAN interface '{}'. {}", iface, e))?;
    }

    // Done with the interface
    let _ = can_iface;

    let mut sock = CanFdSocket::open(iface)
        .map_err(|e| anyhow!("Failed to open CAN interface '{}'. {}", iface, e))?;

    let uses_hw_timestamps = config.plugin.hw_timestamps.unwrap_or(true);
    if uses_hw_timestamps {
        sock.set_timestamps(true)
            .map_err(|e| anyhow!("Failed to enable timestamps. {}", e))?;
    }

    sock.set_filter_accept_all()?;

    let can_filters = config.plugin.filters.as_deref().unwrap_or(&[]);
    if !can_filters.is_empty() {
        sock.set_filters(can_filters)
            .map_err(|e| anyhow!("Failed to set CAN filters. {}", e))?;
    }

    let client = config.connect_and_authenticate().await?;
    info!("Connected to Modality backend");

    let common_timeline_attrs = vec![
        (
            "timeline.modality_can.plugin.version".into(),
            PLUGIN_VERSION.into(),
        ),
        (
            "timeline.modality_can.socketcan.interface".into(),
            iface.into(),
        ),
        (
            "timeline.modality_can.socketcan.hw_timestamp".into(),
            uses_hw_timestamps.into(),
        ),
        (
            "timeline.clock_style".into(),
            if uses_hw_timestamps {
                "relative".into()
            } else {
                "absolute".into()
            },
        ),
    ];
    let mut sender = Sender::new(
        client,
        common_timeline_attrs.into_iter().collect(),
        dbc,
        config,
    );

    let cancel_token = CancellationToken::new();

    let task_tracker = TaskTracker::new();
    let task_cancel_token = cancel_token.clone();
    let mut join_handle: tokio::task::JoinHandle<Result<(), anyhow::Error>> =
        task_tracker.spawn(async move {
            loop {
                tokio::select! {
                    _ = task_cancel_token.cancelled() => {
                        // Task was cancelled
                        sender.close().await?;
                        break;
                    }
                    maybe_res = sock.next() => {
                        if let Some(res) = maybe_res {
                            let (frame, hw_timestamp) = res?;
                            let parsed_frame = parser.parse(&frame, hw_timestamp)?;
                            sender.handle_frame(parsed_frame).await?;
                        } else {
                            break;
                        }
                    }
                }
            }
            Ok(())
        });

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            debug!("User signaled shutdown");
        }
        res = &mut join_handle => {
            match res? {
                Ok(_) => {},
                Err(e) => return Err(e.into()),
            }
        }
    };

    cancel_token.cancel();
    task_tracker.close();
    task_tracker.wait().await;

    Ok(())
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct CanFilter {
    inverted: bool,
    id: u32,
    mask: u32,
}

impl FromStr for CanFilter {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = |input: &str| {
            format!("Invalid CAN filter '{input}'. Expected format is '<id>:<mask>[:!]'.")
        };

        let tokens: Vec<&str> = s.trim().split(':').collect();
        if tokens.len() != 2 && tokens.len() != 3 {
            return Err(err(s));
        }

        let mut inverted = false;
        if tokens.len() == 3 {
            if tokens[2] != "!" {
                return Err(err(s));
            }
            inverted = true;
        }

        let parse_num_or_hex = |input: &str| {
            input
                .parse::<u32>()
                .or_else(|_| u32::from_str_radix(input.trim_start_matches("0x"), 16))
                .map_err(|_| err(s))
        };

        let id = parse_num_or_hex(tokens[0])?;
        let mask = parse_num_or_hex(tokens[1])?;

        Ok(CanFilter { inverted, id, mask })
    }
}

impl From<CanFilter> for socketcan::CanFilter {
    fn from(f: CanFilter) -> Self {
        if f.inverted {
            socketcan::CanFilter::new_inverted(f.id, f.mask)
        } else {
            socketcan::CanFilter::new(f.id, f.mask)
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn can_filters() {
        assert!(CanFilter::from_str("").is_err());
        assert!(CanFilter::from_str("11").is_err());
        assert!(CanFilter::from_str("11:").is_err());
        assert!(CanFilter::from_str("11:22:33").is_err());

        assert_eq!(
            CanFilter::from_str("1:2"),
            Ok(CanFilter {
                inverted: false,
                id: 1,
                mask: 2,
            })
        );
        assert_eq!(
            CanFilter::from_str("22:33:!"),
            Ok(CanFilter {
                inverted: true,
                id: 22,
                mask: 33,
            })
        );

        assert_eq!(
            CanFilter::from_str("0x001:0x002"),
            Ok(CanFilter {
                inverted: false,
                id: 0x001,
                mask: 0x002,
            })
        );
        assert_eq!(
            CanFilter::from_str("0x022:0x033:!"),
            Ok(CanFilter {
                inverted: true,
                id: 0x022,
                mask: 0x033,
            })
        );
    }
}
