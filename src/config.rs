//! Runtime configuration.
//!
//! Paths default to `data/` under the working directory; everything there is
//! rebuildable and gitignored. The SEC User-Agent has no default on purpose —
//! SEC access policy requires a real contact address, and a placeholder would
//! get the IP blocked rather than fail loudly.

use std::collections::HashMap;
use std::env;
use std::path::{Path, PathBuf};

use crate::error::Error;

/// SEC fair-access policy publishes a ceiling of 10 requests/second; we sit
/// under it.
pub const SEC_RATE_LIMIT_PER_SEC: f64 = 8.0;

#[derive(Debug, Clone)]
pub struct Config {
    pub user_agent: String,
    pub data_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub db_path: PathBuf,
}

impl Config {
    /// Resolve configuration from the environment, falling back to a `.env`
    /// file in the working directory. Real environment variables always win.
    pub fn load() -> Result<Self, Error> {
        let dotenv = read_dotenv(Path::new(".env"));
        let get = |key: &str| -> Option<String> {
            env::var(key).ok().or_else(|| dotenv.get(key).cloned())
        };

        let user_agent = get("ASFILED_SEC_USER_AGENT")
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or(Error::MissingUserAgent)?;

        let data_dir = get("ASFILED_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("data"));
        let cache_dir = data_dir.join("cache");
        let db_path = get("ASFILED_DB")
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir.join("asfiled.duckdb"));

        Ok(Self {
            user_agent,
            data_dir,
            cache_dir,
            db_path,
        })
    }

    pub fn ensure_dirs(&self) -> Result<(), Error> {
        std::fs::create_dir_all(&self.cache_dir).map_err(|source| Error::Io {
            path: self.cache_dir.clone(),
            source,
        })
    }
}

/// A three-line `.env` parser rather than a dependency: the file holds a
/// handful of settings and is gitignored, so it never needs to handle
/// anything exotic. Returned as a map — the environment itself is never
/// mutated, which keeps this thread-safe.
fn read_dotenv(path: &Path) -> HashMap<String, String> {
    let mut vars = HashMap::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return vars;
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim_matches('\'');
        vars.insert(key.trim().to_string(), value.to_string());
    }
    vars
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn dotenv_parses_and_ignores_noise() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".env");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, "# comment").unwrap();
        writeln!(f).unwrap();
        writeln!(f, "ASFILED_SEC_USER_AGENT=\"Jane Doe jane@example.com\"").unwrap();
        writeln!(f, "ASFILED_DATA_DIR='/tmp/asfiled-data'").unwrap();
        writeln!(f, "not a kv line").unwrap();

        let vars = read_dotenv(&path);
        assert_eq!(
            vars.get("ASFILED_SEC_USER_AGENT").map(String::as_str),
            Some("Jane Doe jane@example.com")
        );
        assert_eq!(
            vars.get("ASFILED_DATA_DIR").map(String::as_str),
            Some("/tmp/asfiled-data")
        );
        assert_eq!(vars.len(), 2);
    }

    #[test]
    fn dotenv_missing_file_is_empty() {
        assert!(read_dotenv(Path::new("/nonexistent/.env")).is_empty());
    }
}
