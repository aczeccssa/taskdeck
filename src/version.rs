//! Taskdeck release identity shared by the CLI, Web UI, MCP and updater.

use serde::Serialize;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPOSITORY: &str = "aczeccssa/taskdeck";
pub const RELEASES_URL: &str = "https://github.com/aczeccssa/taskdeck/releases";
pub const API_URL: &str = "https://api.github.com/repos/aczeccssa/taskdeck/releases/latest";

#[derive(Debug, Clone, Serialize)]
pub struct VersionInfo {
    pub version: &'static str,
    pub tag: String,
    pub repository: &'static str,
    pub releases_url: &'static str,
    pub target: &'static str,
    pub os: &'static str,
    pub arch: &'static str,
}

pub fn tag() -> String { format!("v{VERSION}") }
pub fn target() -> &'static str {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) { "x86_64-unknown-linux-gnu" }
    else if cfg!(all(target_os = "linux", target_arch = "aarch64")) { "aarch64-unknown-linux-gnu" }
    else if cfg!(all(target_os = "macos", target_arch = "x86_64")) { "x86_64-apple-darwin" }
    else if cfg!(all(target_os = "macos", target_arch = "aarch64")) { "aarch64-apple-darwin" }
    else if cfg!(all(target_os = "windows", target_arch = "x86_64")) { "x86_64-pc-windows-msvc" }
    else if cfg!(all(target_os = "windows", target_arch = "aarch64")) { "aarch64-pc-windows-msvc" }
    else { "unknown" }
}
pub fn os() -> &'static str {
    if cfg!(target_os = "windows") { "windows" } else if cfg!(target_os = "macos") { "macos" } else if cfg!(target_os = "linux") { "linux" } else { "unknown" }
}
pub fn arch() -> &'static str {
    if cfg!(target_arch = "x86_64") { "x86_64" } else if cfg!(target_arch = "aarch64") { "aarch64" } else { "unknown" }
}
pub fn info() -> VersionInfo { VersionInfo { version: VERSION, tag: tag(), repository: REPOSITORY, releases_url: RELEASES_URL, target: target(), os: os(), arch: arch() } }
