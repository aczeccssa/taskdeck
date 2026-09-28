//! Offline Taskdeck YAML validation plus remote schema synchronization.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use jsonschema::Validator;
use semver::Version;
use serde_json::Value;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(windows)]
use std::os::windows::{ffi::OsStrExt, fs::OpenOptionsExt};
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};

use crate::SchemaCommands;

pub(crate) const DEFAULT_SCHEMA_URL: &str = "https://aczeccssa.github.io/taskdeck/schema.json";
const EMBEDDED_SCHEMA: &str = include_str!("../docs-site/public/schema.json");
const SCHEMA_VERSION_KEY: &str = "x-taskdeck-schema-version";
const MAX_SCHEMA_BYTES: usize = 1024 * 1024;
const REMOTE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SchemaSource {
    Remote,
    Cache,
    Bundled,
}

impl SchemaSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::Remote => "remote",
            Self::Cache => "cache",
            Self::Bundled => "bundled",
        }
    }
}

struct SchemaDocument {
    value: Value,
    version: Version,
}

struct DownloadedSchema {
    document: SchemaDocument,
    bytes: Vec<u8>,
}

struct ResolvedSchema {
    document: SchemaDocument,
    source: SchemaSource,
    warnings: Vec<String>,
}

pub(crate) fn run_command(command: SchemaCommands, project: &Path, json: bool) -> Result<()> {
    match command {
        SchemaCommands::Check => check_project(project, json),
        SchemaCommands::Update => update_cache(json),
    }
}

/// Config loads use a valid updated local cache when it is at least as new as
/// the copy compiled into this binary, without performing network access.
pub(crate) fn validate_local_yaml_value(value: &serde_yaml::Value, path: &Path) -> Result<()> {
    let bundled = embedded_schema()?;
    let cached = cache_path()
        .ok()
        .and_then(|cache_path| load_cached_schema(&cache_path).ok());
    let schema = choose_local_schema(bundled, cached);
    validate_yaml_value_against(value, path, &schema)
}

fn validate_yaml_value_against(
    value: &serde_yaml::Value,
    path: &Path,
    schema: &SchemaDocument,
) -> Result<()> {
    let instance = serde_json::to_value(value).with_context(|| {
        format!(
            "{} contains a value that cannot be checked as JSON",
            path.display()
        )
    })?;
    let errors = validation_errors(&schema.value, &instance)?;
    if !errors.is_empty() {
        bail!(
            "{} does not match Taskdeck schema {}:\n  - {}",
            path.display(),
            schema.version,
            errors.join("\n  - ")
        );
    }
    Ok(())
}

fn check_project(project: &Path, json: bool) -> Result<()> {
    let config_path = project.join(crate::config::PROJECT_CONFIG);
    let text = fs::read_to_string(&config_path)
        .with_context(|| format!("failed to read {}", config_path.display()))?;
    let yaml: serde_yaml::Value = serde_yaml::from_str(&text)
        .with_context(|| format!("failed to parse {}", config_path.display()))?;
    let instance = serde_json::to_value(&yaml).with_context(|| {
        format!(
            "{} contains a value that cannot be checked as JSON",
            config_path.display()
        )
    })?;

    let cache_path = cache_path()?;
    let remote_url = remote_url()?;
    let resolved = resolve_schema(&remote_url, &cache_path)?;
    let errors = validation_errors(&resolved.document.value, &instance)?;
    let valid = errors.is_empty();
    let schema_version = resolved.document.version.to_string();
    let schema_source = resolved.source.as_str();
    let result = serde_json::json!({
        "valid": valid,
        "schema_version": schema_version,
        "schema_source": schema_source,
        "project_config": config_path.display().to_string(),
        "errors": errors.clone(),
        "warnings": resolved.warnings.clone(),
    });

    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else if valid {
        println!(
            "{} is valid (schema {}, {}).",
            config_path.display(),
            schema_version,
            schema_source
        );
        for warning in resolved.warnings {
            eprintln!("Warning: {warning}");
        }
    } else {
        eprintln!("{}", config_path.display());
        for error in errors {
            eprintln!("  - {error}");
        }
        eprintln!("Schema {} selected from {}.", schema_version, schema_source);
    }

    if !valid {
        bail!("taskdeck.yaml schema validation failed");
    }
    Ok(())
}

fn update_cache(json: bool) -> Result<()> {
    let cache_path = cache_path()?;
    let remote_url = remote_url()?;
    let version = synchronize_schema(&remote_url, &cache_path)?;
    let result = serde_json::json!({
        "updated": true,
        "schema_version": version.to_string(),
        "schema_source": "remote",
        "cache_path": cache_path,
    });
    if json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("Synchronized schema {version} to {}.", cache_path.display());
    }
    Ok(())
}

fn synchronize_schema(remote_url: &str, cache_path: &Path) -> Result<Version> {
    let downloaded = fetch_remote_schema(remote_url)?;
    let bundled = embedded_schema()?;
    let cached = load_cached_schema(cache_path).ok();
    let highest_local = cached
        .as_ref()
        .map(|schema| &schema.version)
        .into_iter()
        .chain(std::iter::once(&bundled.version))
        .max()
        .expect("bundled schema always exists");
    if &downloaded.document.version < highest_local {
        bail!(
            "remote schema {} is older than the installed schema {}; local cache was not changed",
            downloaded.document.version,
            highest_local
        );
    }

    write_cache_atomically(&cache_path, &downloaded.bytes)?;
    Ok(downloaded.document.version)
}

fn resolve_schema(remote_url: &str, cache_path: &Path) -> Result<ResolvedSchema> {
    let bundled = embedded_schema()?;
    let cached = load_cached_schema(cache_path).ok();
    let newest_local = cached
        .as_ref()
        .map(|schema| &schema.version)
        .into_iter()
        .chain(std::iter::once(&bundled.version))
        .max()
        .expect("bundled schema always exists");
    let mut warnings = Vec::new();

    match fetch_remote_schema(remote_url) {
        Ok(remote) if &remote.document.version >= newest_local => {
            return Ok(ResolvedSchema {
                document: remote.document,
                source: SchemaSource::Remote,
                warnings,
            });
        }
        Ok(remote) => warnings.push(format!(
            "remote schema {} is older than the local schema {}; using a local copy",
            remote.document.version, newest_local
        )),
        Err(error) => warnings.push(format!("remote schema unavailable: {error:#}")),
    }

    if let Some(cached) = cached.filter(|schema| schema.version >= bundled.version) {
        return Ok(ResolvedSchema {
            document: cached,
            source: SchemaSource::Cache,
            warnings,
        });
    }
    Ok(ResolvedSchema {
        document: bundled,
        source: SchemaSource::Bundled,
        warnings,
    })
}

fn choose_local_schema(bundled: SchemaDocument, cached: Option<SchemaDocument>) -> SchemaDocument {
    cached
        .filter(|schema| schema.version >= bundled.version)
        .unwrap_or(bundled)
}

fn remote_url() -> Result<String> {
    let url =
        std::env::var("TASKDECK_SCHEMA_URL").unwrap_or_else(|_| DEFAULT_SCHEMA_URL.to_owned());
    if url.len() > 2048 || !(url.starts_with("https://") || url.starts_with("http://")) {
        bail!("TASKDECK_SCHEMA_URL must be an absolute HTTP or HTTPS URL");
    }
    Ok(url)
}

fn cache_path() -> Result<PathBuf> {
    Ok(crate::daemon::GlobalPaths::discover()?
        .root
        .join("schema.json"))
}

fn embedded_schema() -> Result<SchemaDocument> {
    let value: Value = serde_json::from_str(EMBEDDED_SCHEMA)
        .context("embedded Taskdeck schema is not valid JSON")?;
    validate_schema_document(&value).context("embedded Taskdeck schema is invalid")
}

fn load_cached_schema(path: &Path) -> Result<SchemaDocument> {
    let bytes = read_limited(path)?;
    let value: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("{} is not valid JSON", path.display()))?;
    validate_schema_document(&value)
        .with_context(|| format!("{} is not a valid Taskdeck schema", path.display()))
}

fn fetch_remote_schema(url: &str) -> Result<DownloadedSchema> {
    let agent = ureq::AgentBuilder::new().timeout(REMOTE_TIMEOUT).build();
    let response = agent
        .get(url)
        .set("Accept", "application/schema+json, application/json")
        .set(
            "User-Agent",
            concat!("taskdeck/", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .map_err(|error| {
            let detail = match error {
                ureq::Error::Status(code, _) => format!("HTTP {code}"),
                ureq::Error::Transport(_) => "network or timeout error".to_owned(),
            };
            anyhow::anyhow!("remote schema fetch failed ({detail})")
        })?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take((MAX_SCHEMA_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .context("failed to read remote schema response")?;
    if bytes.len() > MAX_SCHEMA_BYTES {
        bail!("remote schema exceeds the 1 MiB size limit");
    }
    let value: Value = serde_json::from_slice(&bytes).context("remote schema is not valid JSON")?;
    let document = validate_schema_document(&value)
        .context("remote response is not a valid Taskdeck schema")?;
    Ok(DownloadedSchema { document, bytes })
}

fn validate_schema_document(value: &Value) -> Result<SchemaDocument> {
    if value.get("$schema").and_then(Value::as_str)
        != Some("http://json-schema.org/draft-07/schema#")
    {
        bail!("schema must declare JSON Schema Draft 7");
    }
    if value.get("$id").and_then(Value::as_str) != Some(DEFAULT_SCHEMA_URL) {
        bail!("schema $id must be {DEFAULT_SCHEMA_URL}");
    }
    let version = value
        .get(SCHEMA_VERSION_KEY)
        .and_then(Value::as_str)
        .context("schema is missing x-taskdeck-schema-version")?;
    let version = Version::parse(version).context("schema version must be semantic versioning")?;
    let _validator = jsonschema::validator_for(value).context("schema cannot be compiled")?;
    Ok(SchemaDocument {
        value: value.clone(),
        version,
    })
}

fn validation_errors(schema: &Value, instance: &Value) -> Result<Vec<String>> {
    let validator: Validator =
        jsonschema::validator_for(schema).context("selected schema cannot be compiled")?;
    let errors = validator
        .iter_errors(instance)
        .take(50)
        .map(|error| {
            let path = error.instance_path.to_string();
            let path = if path.is_empty() { "/" } else { &path };
            format!("{path}: value does not satisfy the schema")
        })
        .collect();
    Ok(errors)
}

fn write_cache_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("schema cache path has no parent")?;
    fs::create_dir_all(parent).with_context(|| {
        format!(
            "failed to create schema cache directory {}",
            parent.display()
        )
    })?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp_path = parent.join(format!("schema.{nonce}.tmp"));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    #[cfg(windows)]
    options.share_mode(0);
    let result = (|| -> Result<()> {
        let mut file = options
            .open(&temp_path)
            .with_context(|| format!("failed to create {}", temp_path.display()))?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        replace_file(&temp_path, path)?;
        sync_parent_directory(parent)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    result
}

fn read_limited(path: &Path) -> Result<Vec<u8>> {
    let file =
        fs::File::open(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut bytes = Vec::new();
    file.take((MAX_SCHEMA_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_SCHEMA_BYTES {
        bail!("{} exceeds the 1 MiB schema size limit", path.display());
    }
    Ok(bytes)
}

#[cfg(not(windows))]
fn replace_file(source: &Path, target: &Path) -> Result<()> {
    fs::rename(source, target).with_context(|| format!("failed to replace {}", target.display()))
}

#[cfg(windows)]
fn replace_file(source: &Path, target: &Path) -> Result<()> {
    let target_display = target.display().to_string();
    let source_wide: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let target_wide: Vec<u16> = target.as_os_str().encode_wide().chain(Some(0)).collect();
    let succeeded = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            target_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if succeeded == 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("failed to replace {target_display}"));
    }
    Ok(())
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> Result<()> {
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    fn serve_schema(body: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 1024];
            let _ = stream.read(&mut request);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });
        format!("http://{address}/schema.json")
    }

    #[test]
    fn bundled_schema_has_independent_version_and_validates_config() {
        let schema = embedded_schema().unwrap();
        assert_eq!(schema.version, Version::parse("1.0.0").unwrap());
        let yaml: serde_yaml::Value = serde_yaml::from_str(
            "$schema: https://aczeccssa.github.io/taskdeck/schema.json\nversion: 1\nsession: demo\ntasks:\n  api:\n    command: cargo\n    args: [run]\n",
        )
        .unwrap();
        validate_yaml_value_against(&yaml, Path::new("taskdeck.yaml"), &schema).unwrap();

        let invalid: serde_yaml::Value =
            serde_yaml::from_str("version: 1\ntasks:\n  api:\n    stop_timeout_ms: 0\n").unwrap();
        let error = validate_yaml_value_against(&invalid, Path::new("taskdeck.yaml"), &schema)
            .unwrap_err()
            .to_string();
        assert!(error.contains("/tasks/api/stop_timeout_ms"));
    }

    #[test]
    fn local_validation_prefers_a_valid_updated_cache_and_falls_back_to_bundle() {
        let bundled = embedded_schema().unwrap();
        let mut newer_value: Value = serde_json::from_str(EMBEDDED_SCHEMA).unwrap();
        newer_value[SCHEMA_VERSION_KEY] = Value::String("1.1.0".to_owned());
        let newer_cache = validate_schema_document(&newer_value).unwrap();
        assert_eq!(
            choose_local_schema(embedded_schema().unwrap(), Some(newer_cache)).version,
            Version::parse("1.1.0").unwrap()
        );

        let stale_cache =
            validate_schema_document(&serde_json::from_str(EMBEDDED_SCHEMA).unwrap()).unwrap();
        assert_eq!(
            choose_local_schema(bundled, Some(stale_cache)).version,
            Version::parse("1.0.0").unwrap()
        );
        assert_eq!(
            choose_local_schema(embedded_schema().unwrap(), None).version,
            Version::parse("1.0.0").unwrap()
        );
    }

    #[test]
    fn remote_schema_is_preferred_and_cache_is_the_offline_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("schema.json");
        let url = serve_schema(EMBEDDED_SCHEMA);
        let remote = resolve_schema(&url, &cache).unwrap();
        assert_eq!(remote.source, SchemaSource::Remote);
        assert_eq!(remote.document.version, Version::parse("1.0.0").unwrap());

        write_cache_atomically(&cache, EMBEDDED_SCHEMA.as_bytes()).unwrap();
        let cached = resolve_schema("http://127.0.0.1:9/schema.json", &cache).unwrap();
        assert_eq!(cached.source, SchemaSource::Cache);
        assert!(!cached.warnings.is_empty());

        let mut newer: Value = serde_json::from_str(EMBEDDED_SCHEMA).unwrap();
        newer[SCHEMA_VERSION_KEY] = Value::String("1.1.0".to_owned());
        let newer_bytes = serde_json::to_vec(&newer).unwrap();
        write_cache_atomically(&cache, &newer_bytes).unwrap();
        let stale_remote = serve_schema(EMBEDDED_SCHEMA);
        let selected = resolve_schema(&stale_remote, &cache).unwrap();
        assert_eq!(selected.source, SchemaSource::Cache);
        assert_eq!(selected.document.version, Version::parse("1.1.0").unwrap());
    }

    #[test]
    fn bundled_schema_is_used_when_remote_and_cache_are_unavailable() {
        let dir = tempfile::tempdir().unwrap();
        let resolved = resolve_schema(
            "http://127.0.0.1:9/schema.json",
            &dir.path().join("missing.json"),
        )
        .unwrap();
        assert_eq!(resolved.source, SchemaSource::Bundled);
    }

    #[test]
    fn schema_update_validates_before_replacing_the_local_copy() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("schema.json");
        let url = serve_schema(EMBEDDED_SCHEMA);
        let version = synchronize_schema(&url, &cache).unwrap();
        assert_eq!(version, Version::parse("1.0.0").unwrap());
        assert_eq!(fs::read(&cache).unwrap(), EMBEDDED_SCHEMA.as_bytes());

        let original = b"keep the last known good schema";
        fs::write(&cache, original).unwrap();
        let invalid_url = serve_schema("{\"not\":\"a Taskdeck schema\"}");
        assert!(synchronize_schema(&invalid_url, &cache).is_err());
        assert_eq!(fs::read(&cache).unwrap(), original);

        let mut newer: Value = serde_json::from_str(EMBEDDED_SCHEMA).unwrap();
        newer[SCHEMA_VERSION_KEY] = Value::String("1.1.0".to_owned());
        let newer_bytes = serde_json::to_vec(&newer).unwrap();
        write_cache_atomically(&cache, &newer_bytes).unwrap();
        let stale_remote = serve_schema(EMBEDDED_SCHEMA);
        assert!(synchronize_schema(&stale_remote, &cache).is_err());
        assert_eq!(fs::read(&cache).unwrap(), newer_bytes);
    }
}
