//! `board.toml`: server configuration.
//!
//! On first run the file is created with defaults and a fresh
//! `server_secret`. If an existing file has no secret, one is appended,
//! leaving the rest of the file (and its comments) untouched.

use std::fs;
use std::io::Write as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;

/// Length of `server_secret` in bytes, before base64url encoding.
const SECRET_LEN: usize = 32;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Address to listen on. Keep this on localhost and put a TLS proxy in front.
    #[serde(default = "default_bind")]
    pub bind: SocketAddr,
    /// Path to the SQLite database.
    #[serde(default = "default_database")]
    pub database: PathBuf,
    /// The public origin Board is served from, e.g. `https://board.example.com`.
    /// Used for the `Origin` check on writes and for building signed URLs.
    #[serde(default = "default_public_origin")]
    pub public_origin: String,
    /// base64url-encoded 32 random bytes. Signs export and image URLs.
    #[serde(default)]
    pub server_secret: Option<String>,
    /// Typeset base URL, for push-mode exports (phase 7).
    #[serde(default)]
    pub typeset_url: Option<String>,
}

fn default_bind() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 7070))
}

fn default_database() -> PathBuf {
    PathBuf::from("board.db")
}

fn default_public_origin() -> String {
    "http://localhost:7070".to_string()
}

impl Config {
    /// Load the config at `path`, creating it (or adding a missing secret) as needed.
    pub fn load_or_init(path: &Path) -> Result<Self> {
        if !path.exists() {
            write_new(path)?;
            tracing::info!(path = %path.display(), "created config with a new server secret");
        }

        let text = fs::read_to_string(path)
            .with_context(|| format!("reading config {}", path.display()))?;
        let mut config: Config =
            toml::from_str(&text).with_context(|| format!("parsing config {}", path.display()))?;

        if config.server_secret.is_none() {
            let secret = new_secret();
            append_secret(path, &secret)?;
            tracing::info!(path = %path.display(), "added a new server secret to config");
            config.server_secret = Some(secret);
        }

        config.validate()?;
        Ok(config)
    }

    /// The decoded server secret.
    pub fn secret_bytes(&self) -> Result<Vec<u8>> {
        let encoded = self
            .server_secret
            .as_deref()
            .context("server_secret is missing")?;
        let bytes = URL_SAFE_NO_PAD
            .decode(encoded)
            .context("server_secret is not valid base64url")?;
        if bytes.len() < SECRET_LEN {
            bail!("server_secret must be at least {SECRET_LEN} bytes");
        }
        Ok(bytes)
    }

    fn validate(&self) -> Result<()> {
        self.secret_bytes()?;
        if !(self.public_origin.starts_with("http://")
            || self.public_origin.starts_with("https://"))
        {
            bail!("public_origin must start with http:// or https://");
        }
        if self.public_origin.ends_with('/') {
            bail!("public_origin must not end with a slash");
        }
        Ok(())
    }
}

fn new_secret() -> String {
    let mut bytes = [0u8; SECRET_LEN];
    rand::fill(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn write_new(path: &Path) -> Result<()> {
    let text = format!(
        "# Board configuration. See board.example.toml for every option.\n\
         bind = \"{}\"\n\
         database = \"{}\"\n\
         public_origin = \"{}\"\n\
         server_secret = \"{}\"\n",
        default_bind(),
        default_database().display(),
        default_public_origin(),
        new_secret(),
    );
    let mut file = create_private(path)?;
    file.write_all(text.as_bytes())
        .with_context(|| format!("writing config {}", path.display()))?;
    Ok(())
}

fn append_secret(path: &Path, secret: &str) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(path)
        .with_context(|| format!("opening config {}", path.display()))?;
    writeln!(file, "\nserver_secret = \"{secret}\"")
        .with_context(|| format!("writing config {}", path.display()))?;
    Ok(())
}

/// Create a file readable only by its owner: it holds the server secret.
fn create_private(path: &Path) -> Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options
        .open(path)
        .with_context(|| format!("creating config {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_config_with_secret_on_first_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.toml");

        let config = Config::load_or_init(&path).unwrap();
        assert_eq!(config.bind, default_bind());
        assert_eq!(config.secret_bytes().unwrap().len(), SECRET_LEN);

        // The secret is stable across loads.
        let again = Config::load_or_init(&path).unwrap();
        assert_eq!(config.server_secret, again.server_secret);
    }

    #[cfg(unix)]
    #[test]
    fn new_config_is_private() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.toml");
        Config::load_or_init(&path).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn appends_missing_secret_and_keeps_existing_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.toml");
        fs::write(&path, "# mine\nbind = \"127.0.0.1:9000\"\n").unwrap();

        let config = Config::load_or_init(&path).unwrap();
        assert_eq!(config.bind.port(), 9000);
        assert!(config.server_secret.is_some());

        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# mine\n"));
        assert!(text.contains("server_secret = "));
    }

    #[test]
    fn rejects_unknown_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.toml");
        fs::write(&path, "bnid = \"127.0.0.1:9000\"\n").unwrap();
        let err = Config::load_or_init(&path).unwrap_err();
        assert!(format!("{err:#}").contains("unknown field `bnid`"));
    }

    #[test]
    fn rejects_short_secret_and_bad_origin() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("board.toml");

        fs::write(&path, "server_secret = \"c2hvcnQ\"\n").unwrap();
        assert!(Config::load_or_init(&path).is_err());

        let secret = new_secret();
        fs::write(
            &path,
            format!(
                "server_secret = \"{secret}\"\npublic_origin = \"https://board.example.com/\"\n"
            ),
        )
        .unwrap();
        assert!(Config::load_or_init(&path).is_err());
    }
}
