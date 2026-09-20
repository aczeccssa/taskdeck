//! User-owned daemon network configuration.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};

pub(super) const USER_CONFIG_FILE: &str = "taskdeck.json";
const USER_CONFIG_VERSION: u64 = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UserNetworkConfig {
    pub(super) bind_host: String,
    pub(super) web_port: u16,
    pub(super) allow_remote_bind: bool,
    raw: Map<String, Value>,
}

impl UserNetworkConfig {
    pub(super) fn validate(&self) -> Result<()> {
        if self.bind_host.trim().is_empty() {
            bail!("taskdeck.json bind_host cannot be empty");
        }
        if self.web_port == 0 {
            bail!("taskdeck.json web_port must be greater than zero");
        }
        if is_remote_bind_host(&self.bind_host) && !self.allow_remote_bind {
            bail!(
                "taskdeck.json bind_host '{}' is remote; set allow_remote_bind=true to opt in",
                self.bind_host
            );
        }
        Ok(())
    }

    fn from_value(value: Value, path: &Path) -> Result<Self> {
        let raw = value
            .as_object()
            .cloned()
            .with_context(|| format!("{} must contain a JSON object", path.display()))?;
        let version = raw
            .get("version")
            .and_then(Value::as_u64)
            .with_context(|| format!("{} is missing numeric version", path.display()))?;
        if version != USER_CONFIG_VERSION {
            bail!(
                "unsupported {} version {version}; expected {USER_CONFIG_VERSION}",
                path.display()
            );
        }
        let bind_host = raw
            .get("bind_host")
            .and_then(Value::as_str)
            .map(str::to_string)
            .with_context(|| format!("{} is missing string bind_host", path.display()))?;
        let web_port = raw
            .get("web_port")
            .and_then(Value::as_u64)
            .and_then(|value| u16::try_from(value).ok())
            .with_context(|| format!("{} has invalid web_port", path.display()))?;
        // v1 files written before the explicit opt-in field are treated as
        // legacy operator choices and upgraded on the next write.
        let allow_remote_bind = raw
            .get("allow_remote_bind")
            .and_then(Value::as_bool)
            .unwrap_or_else(|| is_remote_bind_host(&bind_host));
        let config = Self {
            bind_host,
            web_port,
            allow_remote_bind,
            raw,
        };
        config.validate()?;
        Ok(config)
    }

    fn to_value(&self) -> Value {
        let mut raw = self.raw.clone();
        raw.insert("version".to_string(), json!(USER_CONFIG_VERSION));
        raw.insert("bind_host".to_string(), json!(self.bind_host));
        raw.insert("web_port".to_string(), json!(self.web_port));
        raw.insert(
            "allow_remote_bind".to_string(),
            json!(self.allow_remote_bind),
        );
        Value::Object(raw)
    }
}

pub(super) fn config_path(root: &Path) -> PathBuf {
    root.join(USER_CONFIG_FILE)
}

pub(super) fn load(root: &Path) -> Result<Option<UserNetworkConfig>> {
    let path = config_path(root);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!(
                "refusing to read symlinked user configuration {}",
                path.display()
            )
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to inspect {}", path.display()));
        }
    }
    let content =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let value = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    let config = UserNetworkConfig::from_value(value, &path)?;
    restrict_permissions(&path)?;
    Ok(Some(config))
}

pub(super) fn load_or_create(root: &Path, legacy: UserNetworkConfig) -> Result<UserNetworkConfig> {
    match load(root)? {
        Some(config) => Ok(config),
        None => {
            legacy.validate()?;
            write(root, &legacy)?;
            let created = load(root)?.context("newly written taskdeck.json was not found")?;
            if created.bind_host != legacy.bind_host || created.web_port != legacy.web_port {
                bail!("newly written taskdeck.json did not round-trip correctly");
            }
            Ok(created)
        }
    }
}

pub(super) fn write(root: &Path, config: &UserNetworkConfig) -> Result<()> {
    config.validate()?;
    let path = config_path(root);
    if let Ok(metadata) = fs::symlink_metadata(&path) {
        if metadata.file_type().is_symlink() {
            bail!(
                "refusing to replace symlinked user configuration {}",
                path.display()
            );
        }
    }
    let content = serde_json::to_vec_pretty(&config.to_value())?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp = root.join(format!(".{USER_CONFIG_FILE}.{nonce}.tmp"));
    let result = (|| -> Result<()> {
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&temp)
            .with_context(|| format!("failed to create {}", temp.display()))?;
        file.write_all(&content)
            .with_context(|| format!("failed to write {}", temp.display()))?;
        file.write_all(b"\n")?;
        file.sync_all()
            .with_context(|| format!("failed to sync {}", temp.display()))?;
        fs::rename(&temp, &path)
            .with_context(|| format!("failed to install {}", path.display()))?;
        sync_parent(root)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

pub(super) fn legacy(bind_host: String, web_port: u16) -> UserNetworkConfig {
    UserNetworkConfig {
        allow_remote_bind: is_remote_bind_host(&bind_host),
        bind_host,
        web_port,
        raw: Map::new(),
    }
}

pub(super) fn is_remote_bind_host(host: &str) -> bool {
    match host.trim() {
        "localhost" | "ip6-localhost" => false,
        value => value
            .parse::<std::net::IpAddr>()
            .map(|address| !address.is_loopback())
            .unwrap_or(true),
    }
}

fn restrict_permissions(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(path, permissions)
            .with_context(|| format!("failed to restrict permissions on {}", path.display()))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn sync_parent(root: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::fs::File;
        File::open(root)
            .with_context(|| format!("failed to open {}", root.display()))?
            .sync_all()
            .with_context(|| format!("failed to sync {}", root.display()))?;
    }
    #[cfg(not(unix))]
    let _ = root;
    Ok(())
}
