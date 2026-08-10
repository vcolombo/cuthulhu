// SPDX-License-Identifier: GPL-3.0-or-later

//! The daemon's configuration: where it listens, and the token that authorizes a
//! client to make a machine move.

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug)]
pub enum ConfigError {
    Unreadable(String),
    Malformed(String),
    NoToken,
    /// The pre-`[tokens]` form. Refused rather than read as an unnamed token: a daemon that
    /// kept working would leave an operator believing they had per-client revocation when they
    /// had one shared key.
    LegacyToken,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Unreadable(m) => write!(f, "the configuration could not be read ({m})"),
            ConfigError::Malformed(m) => write!(f, "the configuration could not be understood ({m})"),
            ConfigError::NoToken =>
                write!(f, "no token is set, so nothing would stop an unknown client starting a cut"),
            ConfigError::LegacyToken => write!(
                f,
                "`token = \"...\"` is no longer read. Give each client its own entry under \
                 [tokens], for example `[tokens]` then `workshop-laptop = \"...\"`, so one can \
                 be revoked without locking out the rest"
            ),
        }
    }
}
impl std::error::Error for ConfigError {}

#[derive(Deserialize)]
struct ConfigFile {
    bind: Option<String>,
    /// Only read so the old single-token form can be refused by name rather than ignored.
    token: Option<String>,
    tokens: Option<BTreeMap<String, String>>,
    max_frame: Option<usize>,
    cert_dir: Option<PathBuf>,
}

pub struct Config {
    pub bind: SocketAddr,
    /// Named per client, so revoking one desktop leaves the others working. A `BTreeMap` rather
    /// than a `HashMap` so the daemon's startup log lists them in a stable order.
    pub tokens: BTreeMap<String, String>,
    pub max_frame: usize,
    pub cert_dir: PathBuf,
}

/// Hand-written rather than derived: `tokens` holds the secrets that authorize a client to
/// make a blade move, and a derived `Debug` would print them verbatim into whatever log or
/// panic message formatted a `Config`. The names are the useful half for debugging anyway —
/// they are what an operator matches against `journalctl` when deciding which line to revoke.
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("bind", &self.bind)
            .field("tokens", &self.tokens.keys().collect::<Vec<_>>())
            .field("max_frame", &self.max_frame)
            .field("cert_dir", &self.cert_dir)
            .finish()
    }
}

/// `toml::de::Error`'s own `Display` renders the offending source line verbatim, so a stray
/// quote on a token line would print that token straight to the operator's log (the daemon
/// prints load errors to stderr, which under the unit is the journal). `span()` only gives a
/// byte range, not the line/column an operator can act on, so keep the first line of `Display`
/// instead — "TOML parse error at line N, column M" — and drop the snippet that follows it.
fn describe(error: toml::de::Error) -> String {
    error.to_string().lines().next().unwrap_or_default().to_string()
}

impl Config {
    pub fn load(path: &Path) -> Result<Config, ConfigError> {
        let text = std::fs::read_to_string(path).map_err(|e| ConfigError::Unreadable(e.to_string()))?;
        let file: ConfigFile = toml::from_str(&text).map_err(|e| ConfigError::Malformed(describe(e)))?;

        if file.token.is_some() {
            return Err(ConfigError::LegacyToken);
        }
        let tokens: BTreeMap<String, String> = file
            .tokens
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, value)| !value.is_empty())
            .collect();
        if tokens.is_empty() {
            return Err(ConfigError::NoToken);
        }
        let bind = file
            .bind
            .unwrap_or_else(|| "127.0.0.1:7878".into())
            .parse()
            .map_err(|e| ConfigError::Malformed(format!("bind: {e}")))?;

        Ok(Config {
            bind,
            tokens,
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
    fn a_config_reads_its_bind_and_tokens() {
        let dir = write_config("bind = \"192.168.1.10:7878\"\n\n[tokens]\nclient = \"s3cret\"\n");
        let config = Config::load(&dir.path().join("cutd.toml")).unwrap();
        assert_eq!(config.bind.port(), 7878);
        assert_eq!(config.tokens.get("client").map(String::as_str), Some("s3cret"));
        assert_eq!(config.max_frame, crate::frame::DEFAULT_MAX_FRAME, "an unset cap takes the default");
    }

    /// The daemon binds LAN-only by default, and this is the predicate that
    /// decides. A machine that can be told to move must not be reachable from
    /// the internet because a config file said `0.0.0.0`.
    #[test]
    fn only_private_addresses_count_as_private() {
        for private in ["127.0.0.1:1", "192.168.1.10:1", "10.0.0.4:1", "172.16.5.5:1"] {
            let dir = write_config(&format!("bind = \"{private}\"\n\n[tokens]\nx = \"t\"\n"));
            assert!(Config::load(&dir.path().join("cutd.toml")).unwrap().is_private_bind(), "{private}");
        }
        for public in ["0.0.0.0:1", "8.8.8.8:1", "172.32.0.1:1"] {
            let dir = write_config(&format!("bind = \"{public}\"\n\n[tokens]\nx = \"t\"\n"));
            assert!(!Config::load(&dir.path().join("cutd.toml")).unwrap().is_private_bind(), "{public}");
        }
    }

    #[test]
    fn a_config_reads_a_table_of_named_tokens() {
        let dir = write_config(
            "bind = \"127.0.0.1:7878\"\n\n[tokens]\nworkshop-laptop = \"aaa\"\noffice-desktop = \"bbb\"\n",
        );
        let config = Config::load(&dir.path().join("cutd.toml")).unwrap();
        assert_eq!(config.tokens.get("workshop-laptop").map(String::as_str), Some("aaa"));
        assert_eq!(config.tokens.get("office-desktop").map(String::as_str), Some("bbb"));
    }

    /// Refused rather than accepted as an unnamed token: a daemon that kept working would leave
    /// an operator believing they had per-client revocation when they had one shared key.
    #[test]
    fn the_old_single_token_form_is_refused_by_name() {
        let dir = write_config("bind = \"127.0.0.1:7878\"\ntoken = \"s3cret\"\n");
        match Config::load(&dir.path().join("cutd.toml")) {
            Err(ConfigError::LegacyToken) => {}
            other => panic!("expected LegacyToken, got {other:?}"),
        }
    }

    #[test]
    fn the_refusal_names_the_form_to_use_instead() {
        let message = ConfigError::LegacyToken.to_string();
        assert!(message.contains("[tokens]"), "the message must name the replacement: {message}");
    }

    #[test]
    fn a_config_with_no_tokens_at_all_is_refused() {
        let dir = write_config("bind = \"127.0.0.1:7878\"\n");
        assert!(matches!(Config::load(&dir.path().join("cutd.toml")), Err(ConfigError::NoToken)));

        let empty = write_config("bind = \"127.0.0.1:7878\"\n\n[tokens]\n");
        assert!(matches!(Config::load(&empty.path().join("cutd.toml")), Err(ConfigError::NoToken)));
    }

    /// An empty value would authorize everyone that guessed an empty string.
    #[test]
    fn a_token_with_an_empty_value_is_refused() {
        let dir = write_config("bind = \"127.0.0.1:7878\"\n\n[tokens]\nlaptop = \"\"\n");
        assert!(matches!(Config::load(&dir.path().join("cutd.toml")), Err(ConfigError::NoToken)));
    }

    /// The other way a token can reach the journal: toml's own Display renders the
    /// offending source line, and the daemon prints load errors to stderr.
    #[test]
    fn a_malformed_config_reports_where_without_quoting_the_line() {
        let dir = write_config("bind = \"127.0.0.1:7878\"\n\n[tokens]\nlaptop = \"sup3rs3cret\n");
        let message = match Config::load(&dir.path().join("cutd.toml")) {
            Err(e) => e.to_string(),
            Ok(_) => panic!("an unterminated quote must not parse"),
        };
        assert!(!message.contains("sup3rs3cret"), "a token value reached the error: {message}");
        assert!(message.contains("line"), "an operator still needs to be told where: {message}");
    }

    /// A derived `Debug` here would print every client's token. The names are safe and useful;
    /// the values are neither.
    #[test]
    fn debug_shows_which_clients_exist_and_never_their_tokens() {
        let dir = write_config(
            "bind = \"127.0.0.1:7878\"\n\n[tokens]\nworkshop-laptop = \"sup3rs3cret\"\n",
        );
        let config = Config::load(&dir.path().join("cutd.toml")).unwrap();
        let shown = format!("{config:?}");
        assert!(shown.contains("workshop-laptop"), "the client names are the useful half: {shown}");
        assert!(!shown.contains("sup3rs3cret"), "a token value reached a Debug string: {shown}");
    }
}
