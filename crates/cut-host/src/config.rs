// SPDX-License-Identifier: GPL-3.0-or-later

//! The daemon's configuration: where it listens, and the token that authorizes a
//! client to make a machine move.

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug)]
pub enum ConfigError {
    Unreadable(String),
    Malformed(String),
    NoToken,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Unreadable(m) => write!(f, "the configuration could not be read ({m})"),
            ConfigError::Malformed(m) => write!(f, "the configuration could not be understood ({m})"),
            ConfigError::NoToken =>
                write!(f, "no token is set, so nothing would stop an unknown client starting a cut"),
        }
    }
}
impl std::error::Error for ConfigError {}

#[derive(Deserialize)]
struct ConfigFile {
    bind: Option<String>,
    token: Option<String>,
    max_frame: Option<usize>,
    cert_dir: Option<PathBuf>,
}

pub struct Config {
    pub bind: SocketAddr,
    pub token: String,
    pub max_frame: usize,
    pub cert_dir: PathBuf,
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|e| ConfigError::Unreadable(e.to_string()))?;
        let file: ConfigFile = toml::from_str(&text).map_err(|e| ConfigError::Malformed(e.to_string()))?;

        let token = file.token.unwrap_or_default();
        if token.is_empty() {
            return Err(ConfigError::NoToken);
        }
        let bind = file
            .bind
            .unwrap_or_else(|| "0.0.0.0:7878".into())
            .parse()
            .map_err(|e| ConfigError::Malformed(format!("bind: {e}")))?;

        Ok(Config {
            bind,
            token,
            max_frame: file.max_frame.unwrap_or(crate::frame::DEFAULT_MAX_FRAME),
            cert_dir: file.cert_dir.unwrap_or_else(|| PathBuf::from("/var/lib/cuthulhu")),
        })
    }

    /// Whether this address is on a private network. The daemon refuses a public
    /// bind unless told otherwise, because the thing on the other end of it can be
    /// made to move a blade.
    pub fn is_private_bind(&self) -> bool {
        match self.bind.ip() {
            IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
            IpAddr::V6(v6) => v6.is_loopback() || v6.is_unique_local(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_config(body: &str) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("cutd.toml"), body).unwrap();
        dir
    }

    #[test]
    fn a_config_reads_its_bind_and_token() {
        let dir = write_config("bind = \"192.168.1.10:7878\"\ntoken = \"s3cret\"\n");
        let config = Config::load(&dir.path().join("cutd.toml")).unwrap();
        assert_eq!(config.bind.port(), 7878);
        assert_eq!(config.token, "s3cret");
        assert_eq!(config.max_frame, crate::frame::DEFAULT_MAX_FRAME, "an unset cap takes the default");
    }

    #[test]
    fn a_config_without_a_token_is_refused() {
        let dir = write_config("bind = \"127.0.0.1:7878\"\n");
        assert!(matches!(Config::load(&dir.path().join("cutd.toml")), Err(ConfigError::NoToken)));
    }

    /// An empty token would authorize everyone. It is worth its own refusal
    /// because it is what a half-finished config file leaves behind.
    #[test]
    fn an_empty_token_is_refused() {
        let dir = write_config("bind = \"127.0.0.1:7878\"\ntoken = \"\"\n");
        assert!(matches!(Config::load(&dir.path().join("cutd.toml")), Err(ConfigError::NoToken)));
    }

    /// The daemon binds LAN-only by default, and this is the predicate that
    /// decides. A machine that can be told to move must not be reachable from
    /// the internet because a config file said `0.0.0.0`.
    #[test]
    fn only_private_addresses_count_as_private() {
        for private in ["127.0.0.1:1", "192.168.1.10:1", "10.0.0.4:1", "172.16.5.5:1"] {
            let dir = write_config(&format!("bind = \"{private}\"\ntoken = \"t\"\n"));
            assert!(Config::load(&dir.path().join("cutd.toml")).unwrap().is_private_bind(), "{private}");
        }
        for public in ["0.0.0.0:1", "8.8.8.8:1", "172.32.0.1:1"] {
            let dir = write_config(&format!("bind = \"{public}\"\ntoken = \"t\"\n"));
            assert!(!Config::load(&dir.path().join("cutd.toml")).unwrap().is_private_bind(), "{public}");
        }
    }
}
