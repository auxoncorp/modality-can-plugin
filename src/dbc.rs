use anyhow::anyhow;
use can_dbc::DBC;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use tracing::{info, warn};

#[derive(Debug)]
pub struct Dbc {
    pub file_name: Option<String>,
    pub sha256: String,
    pub inner: DBC,
}

impl Dbc {
    pub fn from_file<P: AsRef<Path>>(p: P) -> Result<Self, anyhow::Error> {
        let path_display = p.as_ref().display();
        info!(dbc = %path_display, "Reading DBC file");
        let file_name = p
            .as_ref()
            .file_name()
            .map(|n| n.to_string_lossy().to_string());
        let content = fs::read_to_string(p.as_ref())
            .map_err(|e| anyhow!("Failed to read DBC file '{}'. {}", path_display, e))?;
        let mut hasher = Sha256::new();
        hasher.update(&content);
        let sha256 = format!("{:x}", hasher.finalize());
        let can_dbc = match DBC::try_from(content.as_str()) {
            Ok(dbc) => dbc,
            Err(can_dbc::Error::Incomplete(dbc, _)) => {
                warn!(
                    dbc = %path_display,
                    "DBC file was partially read and may be incomplete"
                );
                dbc
            }
            Err(can_dbc::Error::Nom(e)) => {
                return Err(anyhow!(
                    "Failed to read the DBC file '{path_display}' due to a parser error. {e}"
                ));
            }
            Err(can_dbc::Error::MultipleMultiplexors) => {
                return Err(anyhow!("Failed to read the DBC file '{path_display}' due to unsupported extended multimultiplexing"));
            }
        };
        Ok(Self {
            file_name,
            sha256,
            inner: can_dbc,
        })
    }
}

// Several things in the DBC use empty strings as empty values
pub(crate) trait EmptyStringExt {
    fn empty_opt(&self) -> Option<&str>;
}

impl EmptyStringExt for &String {
    fn empty_opt(&self) -> Option<&str> {
        if self.is_empty() {
            None
        } else {
            Some(self.as_str())
        }
    }
}
