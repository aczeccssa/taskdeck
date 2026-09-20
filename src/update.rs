//! GitHub Release discovery and safe self-update helpers.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::daemon;
use crate::version;

const CHECK_INTERVAL_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseAsset { pub name: String, pub browser_download_url: String, pub size: u64 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub tag_name: String,
    pub html_url: String,
    pub published_at: Option<String>,
    pub body: Option<String>,
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateStatus {
    pub enabled: bool,
    pub current_version: String,
    pub latest_version: Option<String>,
    pub available: bool,
    pub checked_at_ms: Option<u64>,
    pub release_url: Option<String>,
    pub error: Option<String>,
    pub asset_name: Option<String>,
}

pub fn timestamp_ms() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64 }
pub fn enabled() -> bool { std::env::var("TASKDECK_UPDATE_CHECK").as_deref() != Ok("0") }
pub fn should_check(last_checked_ms: Option<u64>, now_ms: u64) -> bool { enabled() && last_checked_ms.map(|last| now_ms.saturating_sub(last) >= CHECK_INTERVAL_MS).unwrap_or(true) }

pub fn latest() -> Result<ReleaseInfo> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(10)).build();
    let api_url = std::env::var("TASKDECK_UPDATE_API_URL").unwrap_or_else(|_| version::API_URL.to_string());
    agent.get(&api_url).set("User-Agent", "taskdeck-updater").call()
        .context("GitHub release request failed")?.into_json().context("invalid GitHub release response")
}

pub fn parse_version(tag: &str) -> Result<semver::Version> {
    semver::Version::parse(tag.strip_prefix('v').unwrap_or(tag)).with_context(|| format!("invalid release version '{tag}'"))
}

pub fn select_asset(release: &ReleaseInfo) -> Result<&ReleaseAsset> {
    let suffix = if cfg!(target_os = "windows") { ".zip" } else { ".tar.gz" };
    let needle = format!("{}-{}", release.tag_name.trim_start_matches('v'), version::target());
    release.assets.iter().find(|asset| asset.name.contains(&needle) && asset.name.ends_with(suffix))
        .with_context(|| format!("no release asset for {} ({})", version::target(), suffix))
}

pub fn status_from_release(release: ReleaseInfo, checked_at_ms: u64) -> UpdateStatus {
    let current = semver::Version::parse(version::VERSION).ok();
    let latest = parse_version(&release.tag_name).ok();
    let available = matches!((&current, &latest), (Some(current), Some(latest)) if latest > current);
    let asset_name = select_asset(&release).ok().map(|asset| asset.name.clone());
    UpdateStatus { enabled: enabled(), current_version: version::VERSION.to_string(), latest_version: latest.map(|v| v.to_string()), available, checked_at_ms: Some(checked_at_ms), release_url: Some(release.html_url), error: None, asset_name }
}

pub fn check() -> UpdateStatus {
    let checked = timestamp_ms();
    match latest() {
        Ok(release) => status_from_release(release, checked),
        Err(error) => UpdateStatus { enabled: enabled(), current_version: version::VERSION.to_string(), latest_version: None, available: false, checked_at_ms: Some(checked), release_url: None, error: Some(format!("{error:#}")), asset_name: None },
    }
}

pub fn download_and_stage(release: &ReleaseInfo, root: &Path) -> Result<PathBuf> {
    let asset = select_asset(release)?;
    let updates = root.join("updates");
    fs::create_dir_all(&updates)?;
    let archive = updates.join(&asset.name);
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(60)).build();
    let response = agent.get(&asset.browser_download_url).set("User-Agent", "taskdeck-updater").call().context("release asset download failed")?;
    let mut reader = response.into_reader();
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut reader, &mut bytes)?;
    fs::write(&archive, &bytes)?;
    let releases_url = std::env::var("TASKDECK_RELEASES_URL").unwrap_or_else(|_| version::RELEASES_URL.to_string());
    let sums_url = format!("{}/download/{}/SHA256SUMS", releases_url.trim_end_matches('/'), release.tag_name);
    let sums = agent.get(&sums_url).set("User-Agent", "taskdeck-updater").call().context("checksum download failed")?.into_string()?;
    let expected = sums.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let digest = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        (name == asset.name).then_some(digest.to_string())
    }).context("release checksum does not contain selected asset")?;
    let actual = format!("{:x}", Sha256::digest(&bytes));
    if actual != expected { bail!("checksum mismatch for {}", asset.name); }
    Ok(archive)
}

pub async fn install_latest_release(release: ReleaseInfo) -> Result<String> {
    let root = daemon::root_path()?;
    tokio::task::spawn_blocking(move || install_release(&release, &root)).await?
}

fn install_release(release: &ReleaseInfo, root: &Path) -> Result<String> {
    let archive = download_and_stage(release, root)?;
    let target = std::env::current_exe().context("cannot locate current executable")?;
    let staged = extract_binary(&archive, root)?;
    let helper = root.join("updates").join(if cfg!(windows) { "apply-update.cmd" } else { "apply-update.sh" });
    write_helper(&helper, &staged, &target, std::process::id())?;
    if cfg!(windows) { Command::new("cmd").args(["/C", helper.to_string_lossy().as_ref()]).spawn()?; }
    else { Command::new("sh").arg(&helper).spawn()?; }
    Ok(format!("update downloaded and queued for installation: {}", staged.display()))
}

fn extract_binary(archive: &Path, root: &Path) -> Result<PathBuf> {
    let destination = root.join("updates").join(if cfg!(windows) { "taskdeck.new.exe" } else { "taskdeck.new" });
    if destination.exists() { fs::remove_file(&destination)?; }
    let updates = root.join("updates");
    if cfg!(windows) {
        let status = Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &format!("Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force", archive.display(), updates.display())]).status()?;
        if !status.success() { bail!("failed to extract Windows release archive"); }
        fs::rename(updates.join("taskdeck.exe"), &destination)?;
    } else {
        let status = Command::new("tar").args(["-xzf", &archive.to_string_lossy(), "-C", &updates.to_string_lossy()]).status()?;
        if !status.success() { bail!("failed to extract release archive"); }
        fs::rename(updates.join("taskdeck"), &destination)?;
    }
    Ok(destination)
}

fn write_helper(path: &Path, staged: &Path, target: &Path, parent_pid: u32) -> Result<()> {
    let content = if cfg!(windows) {
        format!("@echo off\r\ntimeout /t 2 /nobreak >nul\r\ntaskkill /PID {parent_pid} /T /F >nul 2>&1\r\ntimeout /t 1 /nobreak >nul\r\nmove /Y \"{}\" \"{}\" >nul\r\nstart \"\" \"{}\" daemon --background\r\ndel \"%~f0\"\r\n", staged.display(), target.display(), target.display())
    } else {
        format!("#!/bin/sh\nsleep 2\nkill -TERM {parent_pid} 2>/dev/null || true\nsleep 1\nmv -f '{}' '{}.bak'\nmv -f '{}' '{}'\n'{}' daemon --background >/dev/null 2>&1 &\nrm -- '$0'\n", target.display(), target.display(), staged.display(), target.display(), target.display())
    };
    fs::write(path, content)?;
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compares_release_versions_and_selects_current_target() {
        assert!(parse_version("v1.2.3").unwrap() > parse_version("v1.2.2").unwrap());
        let release = ReleaseInfo {
            tag_name: "v9.9.9".to_string(), html_url: "https://example.invalid".to_string(),
            published_at: None, body: None,
            assets: vec![ReleaseAsset { name: format!("taskdeck-v9.9.9-{}{}", version::target(), if cfg!(target_os = "windows") { ".zip" } else { ".tar.gz" }), browser_download_url: "https://example.invalid/a".to_string(), size: 1 }],
        };
        assert_eq!(select_asset(&release).unwrap().size, 1);
    }

    #[test]
    fn invalid_release_version_is_rejected() {
        assert!(parse_version("latest").is_err());
    }
}
