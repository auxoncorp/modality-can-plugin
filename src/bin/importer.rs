use anyhow::anyhow;
use auxon_sdk::plugin_utils::serde::from_str;
use auxon_sdk::{init_tracing, plugin_utils::ingest::Config};
use clap::Parser;
use modality_can::{candump, CanParser, Dbc, HasCommonConfig, Sender, PLUGIN_VERSION};
use serde::{Deserialize, Serialize};
use std::io::BufRead;
use std::{fs::File, io::BufReader, path::PathBuf};
use tracing::{info, warn};

/// Import CAN log files
#[derive(clap::Parser)]
struct ImporterOpts {
    /// File to import (e.g. candump.log)
    file: Option<PathBuf>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
struct ImporterConfig {
    /// Assume the timestamps in the log are absolute.
    /// For hardware timestamps, leave false.
    /// Defaults to false.
    #[serde(deserialize_with = "from_str", alias = "absolute_timestamps")]
    absolute_timestamps: Option<bool>,

    /// File to import (e.g. candump.log)
    #[serde(deserialize_with = "from_str")]
    file: Option<PathBuf>,

    #[serde(flatten)]
    common: modality_can::CommonConfig,
}

impl HasCommonConfig for ImporterConfig {
    fn common_config(&self) -> &modality_can::CommonConfig {
        &self.common
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    init_tracing!();

    let opts = ImporterOpts::parse();

    let config = Config::<ImporterConfig>::load("MODALITY_CAN_")?;

    let dbc = config
        .plugin
        .common
        .dbc
        .as_ref()
        .map(Dbc::from_file)
        .transpose()?;

    let mut parser = CanParser::new(&config.plugin.common, dbc.as_ref().map(|dbc| &dbc.inner))?;

    let log_file_path = opts.file.as_ref().or(config.plugin.file.as_ref()).ok_or_else(||
        anyhow!("Missing input log file. Specify a path to import on the command line or configuration file"))?;

    info!(
        file = %log_file_path.display(),
        "Importing CAN frames from file"
    );

    let file = File::open(log_file_path).map_err(|e| {
        anyhow!(
            "Failed to open log file '{}'. {}",
            log_file_path.display(),
            e
        )
    })?;

    let mut reader = BufReader::new(file);

    let absolute_timestamps = config.plugin.absolute_timestamps.unwrap_or(false);
    let common_timeline_attrs = vec![
        (
            "timeline.modality_can.plugin.version".into(),
            PLUGIN_VERSION.into(),
        ),
        (
            "timeline.clock_style".into(),
            if absolute_timestamps {
                "absolute".into()
            } else {
                "relative".into()
            },
        ),
        (
            "timeline.modality_can.importer.file_name".into(),
            log_file_path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "NA".to_owned())
                .into(),
        ),
    ];

    let client = config.connect_and_authenticate_ingest().await?;
    info!("Connected to Modality backend");

    let mut sender = Sender::new(
        client,
        common_timeline_attrs.into_iter().collect(),
        dbc,
        config,
    );

    let mut frame_count = 0_u64;
    let mut line_buf = String::with_capacity(8 * 1024);
    loop {
        line_buf.clear();
        let line_size = reader.read_line(&mut line_buf)?;
        if line_size == 0 {
            break;
        }

        // Skip if not an entry (comment/etc)
        if !line_buf.starts_with(candump::SOF) {
            continue;
        }

        match candump::parse(&line_buf) {
            Ok((_, (timestamp, _iface, frame))) => {
                let parsed_frame = parser.parse(&frame, Some(timestamp))?;
                sender.handle_frame(parsed_frame).await?;
                frame_count += 1;
            }
            Err(e) => {
                warn!(%e, line = line_buf, "Failed to parse log file line");
            }
        }
    }

    sender.close().await?;

    info!(frame_count, "Finished importing");

    Ok(())
}
