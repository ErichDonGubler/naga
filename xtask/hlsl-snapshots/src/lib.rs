use std::{fs, io, path::Path};

use anyhow::{anyhow, ensure};
use nanoserde::{self, DeRon, DeRonErr, SerRon};

#[derive(Debug, thiserror::Error)]
#[error("failed to read RON configuration of HLSL snapshot test")]
struct BadRonParse(#[source] BadRonParseKind);

#[derive(Debug, thiserror::Error)]
enum BadRonParseKind {
    #[error(transparent)]
    Read { source: io::Error },
    #[error(transparent)]
    Parse { source: DeRonErr },
    #[error("no configuration was specified")]
    Empty,
}

#[derive(Debug, DeRon, SerRon)]
pub struct Config {
    pub vertex: Vec<ConfigItem>,
    pub fragment: Vec<ConfigItem>,
    pub compute: Vec<ConfigItem>,
}

impl Config {
    pub fn empty() -> Self {
        Self {
            vertex: Default::default(),
            fragment: Default::default(),
            compute: Default::default(),
        }
    }

    pub fn from_path(path: impl AsRef<Path>) -> anyhow::Result<Config> {
        let path = path.as_ref();
        let raw_config = fs::read_to_string(path)
            .map_err(|source| BadRonParse(BadRonParseKind::Read { source }))?;
        let config = Config::deserialize_ron(&raw_config)
            .map_err(|source| BadRonParse(BadRonParseKind::Parse { source }))?;
        ensure!(!config.is_empty(), BadRonParse(BadRonParseKind::Empty));
        Ok(config)
    }

    pub fn to_file(&self, path: impl AsRef<Path>) -> anyhow::Result<()> {
        let path = path.as_ref();
        let mut s = self.serialize_ron();
        s.push('\n');
        fs::write(path, &s).map_err(|e| anyhow!("failed to write to {}: {e}", path.display()))
    }

    pub fn is_empty(&self) -> bool {
        let Self {
            vertex,
            fragment,
            compute,
        } = self;
        vertex.is_empty() && fragment.is_empty() && compute.is_empty()
    }
}

#[derive(Debug, DeRon, SerRon)]
pub struct ConfigItem {
    pub entry_point: String,
    /// See also
    /// <https://learn.microsoft.com/en-us/windows/win32/direct3dtools/dx-graphics-tools-fxc-using>.
    pub target_profile: String,
}
