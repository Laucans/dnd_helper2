//! Configuration read from the process environment, and nowhere else.
//!
//! A connection string is a secret: it is held in [`Secret`], which never
//! prints its value, and an error names the variable, never what it holds.

use std::fmt;
use std::str::FromStr;

use sqlx::postgres::PgConnectOptions;

pub const DEFAULT_PORT: u16 = 7878;

pub const DATABASE_URL: &str = "DATABASE_URL";
pub const DATABASE_URL_READONLY: &str = "DATABASE_URL_READONLY";
pub const SERVER_PORT: &str = "SERVER_PORT";

/// A value that must never reach a log, a file or a response.
pub struct Secret(String);

impl Secret {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret([redacted])")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

#[derive(Debug)]
pub struct Config {
    pub database_url: Secret,
    /// Not needed to start the server; never a fallback for `database_url`.
    pub database_url_readonly: Option<Secret>,
    pub port: u16,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ConfigError {
    /// Unset, empty or whitespace-only.
    #[error("{0} is not set")]
    Missing(&'static str),
    #[error("{0} is malformed")]
    Malformed(&'static str),
    /// Set, but not an integer in 1..=65535. Blank means unset.
    #[error("{0} is not a valid port")]
    InvalidPort(&'static str),
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    /// Reads the configuration through `get`, so tests never touch the
    /// process environment.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let non_blank = |name: &str| get(name).filter(|v| !v.trim().is_empty());

        let database_url = non_blank(DATABASE_URL).ok_or(ConfigError::Missing(DATABASE_URL))?;
        parse_url(&database_url, DATABASE_URL)?;

        let database_url_readonly = match non_blank(DATABASE_URL_READONLY) {
            Some(url) => {
                parse_url(&url, DATABASE_URL_READONLY)?;
                Some(Secret(url))
            }
            None => None,
        };

        let port = match non_blank(SERVER_PORT) {
            None => DEFAULT_PORT,
            Some(raw) => match raw.trim().parse::<u16>() {
                Ok(port) if port != 0 => port,
                _ => return Err(ConfigError::InvalidPort(SERVER_PORT)),
            },
        };

        Ok(Self {
            database_url: Secret(database_url),
            database_url_readonly,
            port,
        })
    }

    /// The options of `DATABASE_URL`. `PgConnectOptions` prints its password
    /// in `Debug`: never format it, never store it in a `Debug` type.
    pub fn connect_options(&self) -> Result<PgConnectOptions, ConfigError> {
        parse_url(self.database_url.expose(), DATABASE_URL)
    }
}

fn parse_url(url: &str, name: &'static str) -> Result<PgConnectOptions, ConfigError> {
    // sqlx parses any URL; only a PostgreSQL one names our database.
    let scheme_ok = ["postgres://", "postgresql://"]
        .iter()
        .any(|scheme| url.starts_with(scheme));
    if !scheme_ok {
        return Err(ConfigError::Malformed(name));
    }
    PgConnectOptions::from_str(url).map_err(|_| ConfigError::Malformed(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| map.get(name).cloned()
    }

    const URL: &str = "postgres://leakuser:leakpw-1@leakhost:5432/leakdb";
    const RO_URL: &str = "postgres://rouser:ropw-1@rohost:5432/rodb";

    #[test]
    fn missing_empty_or_blank_database_url_is_missing() {
        for vars in [
            vec![],
            vec![(DATABASE_URL, "")],
            vec![(DATABASE_URL, "  \t ")],
        ] {
            let err = Config::from_lookup(lookup(&vars)).unwrap_err();
            assert_eq!(err, ConfigError::Missing(DATABASE_URL));
            assert_eq!(err.to_string(), "DATABASE_URL is not set");
        }
    }

    #[test]
    fn malformed_database_url_names_the_variable_only() {
        for bad in [
            "mysql://leakuser:leakpw-1@leakhost/leakdb",
            "postgres://leakuser:leakpw-1@leakhost:notaport/leakdb",
            "leakpw-1",
        ] {
            let err = Config::from_lookup(lookup(&[(DATABASE_URL, bad)])).unwrap_err();
            assert_eq!(err, ConfigError::Malformed(DATABASE_URL), "{bad}");
            let shown = format!("{err} {err:?}");
            assert!(shown.contains("DATABASE_URL"));
            assert!(!shown.contains("leak"), "{shown}");
        }
    }

    #[test]
    fn malformed_readonly_url_names_the_readonly_variable_only() {
        let err = Config::from_lookup(lookup(&[
            (DATABASE_URL, URL),
            (
                DATABASE_URL_READONLY,
                "postgres://rouser:ropw-1@rohost:nope/rodb",
            ),
        ]))
        .unwrap_err();
        assert_eq!(err, ConfigError::Malformed(DATABASE_URL_READONLY));
        assert!(!format!("{err} {err:?}").contains("ropw"));
    }

    #[test]
    fn debug_and_display_redact_both_urls() {
        let config = Config::from_lookup(lookup(&[
            (DATABASE_URL, URL),
            (DATABASE_URL_READONLY, RO_URL),
        ]))
        .unwrap();
        let shown = format!(
            "{config:?} {} {:?} {}",
            config.database_url,
            config.database_url,
            config.database_url_readonly.as_ref().unwrap()
        );
        for marker in [
            "leakuser", "leakpw", "leakhost", "leakdb", "rouser", "ropw", "rohost", "rodb",
        ] {
            assert!(!shown.contains(marker), "{marker} leaked in {shown}");
        }
        assert!(shown.contains("[redacted]"));
        assert_eq!(config.database_url.expose(), URL);
    }

    #[test]
    fn port_defaults_and_refuses_zero_and_garbage() {
        let config = Config::from_lookup(lookup(&[(DATABASE_URL, URL)])).unwrap();
        assert_eq!(config.port, DEFAULT_PORT);
        assert!(config.database_url_readonly.is_none());

        let config =
            Config::from_lookup(lookup(&[(DATABASE_URL, URL), (SERVER_PORT, "9001")])).unwrap();
        assert_eq!(config.port, 9001);

        for blank in ["", "  "] {
            let config =
                Config::from_lookup(lookup(&[(DATABASE_URL, URL), (SERVER_PORT, blank)])).unwrap();
            assert_eq!(config.port, DEFAULT_PORT);
        }

        for bad in ["0", "abc", "65536", "-1"] {
            let err = Config::from_lookup(lookup(&[(DATABASE_URL, URL), (SERVER_PORT, bad)]))
                .unwrap_err();
            assert_eq!(err, ConfigError::InvalidPort(SERVER_PORT), "port {bad:?}");
        }
    }

    #[test]
    fn readonly_url_is_never_a_fallback() {
        let err = Config::from_lookup(lookup(&[(DATABASE_URL_READONLY, RO_URL)])).unwrap_err();
        assert_eq!(err, ConfigError::Missing(DATABASE_URL));
    }
}
