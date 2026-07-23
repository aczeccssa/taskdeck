use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::config::TaskSpec;
use crate::protocol::{
    ServiceClassification, ServiceConfidence, ServiceEndpoint, ServiceInspectionState,
    ServiceObservation, TechnologyProfile,
};

pub fn infer_service(spec: &TaskSpec) -> ServiceObservation {
    let command = spec.display_command().to_lowercase();
    let manifest = package_manifest_signals(&spec.cwd);
    let combined = format!("{command} {}", manifest.join(" "));
    let mut evidence = Vec::new();
    let (runtime, framework) = if contains_any(&combined, &["vite"]) {
        evidence.push("vite command or package dependency".to_string());
        (Some("node"), Some("vite"))
    } else if contains_any(&combined, &["next dev", "\"next\""]) {
        evidence.push("Next.js command or package dependency".to_string());
        (Some("node"), Some("next.js"))
    } else if contains_any(&combined, &["nuxt", "\"nuxt\""]) {
        evidence.push("Nuxt command or package dependency".to_string());
        (Some("node"), Some("nuxt"))
    } else if contains_any(&combined, &["uvicorn"]) {
        evidence.push("uvicorn command".to_string());
        (Some("python"), Some("uvicorn"))
    } else if contains_any(&combined, &["gunicorn"]) {
        evidence.push("gunicorn command".to_string());
        (Some("python"), Some("gunicorn"))
    } else if contains_any(&combined, &["django", "manage.py runserver"]) {
        evidence.push("Django runserver command or dependency".to_string());
        (Some("python"), Some("django"))
    } else if contains_any(&combined, &["flask run", "\"flask\""]) {
        evidence.push("Flask command or dependency".to_string());
        (Some("python"), Some("flask"))
    } else if contains_any(&combined, &["dotnet run", "aspnetcore"]) {
        evidence.push("dotnet/ASP.NET command or project".to_string());
        (Some("dotnet"), Some("asp.net core"))
    } else if contains_any(&combined, &["cargo run", "cargo watch"]) {
        evidence.push("Cargo run command".to_string());
        (Some("rust"), None)
    } else if contains_any(&combined, &["go run", "go.mod"]) {
        evidence.push("Go command or module".to_string());
        (Some("go"), None)
    } else if contains_any(&combined, &["spring-boot", "quarkus"]) {
        evidence.push("JVM web framework signal".to_string());
        (
            Some("jvm"),
            Some(if combined.contains("quarkus") {
                "quarkus"
            } else {
                "spring boot"
            }),
        )
    } else if contains_any(&combined, &["node ", "npm ", "pnpm ", "yarn ", "bun "]) {
        evidence.push("Node package runner command".to_string());
        (Some("node"), None)
    } else if contains_any(&combined, &["python", "poetry run", "uv run"]) {
        evidence.push("Python command".to_string());
        (Some("python"), None)
    } else {
        (None, None)
    };

    let command_looks_like_service = contains_any(
        &combined,
        &[
            " dev",
            "serve",
            "server",
            "listen",
            "runserver",
            "uvicorn",
            "gunicorn",
            "flask run",
            "dotnet run",
        ],
    );
    let classification = if framework.is_some() || command_looks_like_service {
        ServiceClassification::Service
    } else if runtime.is_some() {
        ServiceClassification::Process
    } else {
        ServiceClassification::Unknown
    };
    let confidence = if framework.is_some() {
        ServiceConfidence::High
    } else if runtime.is_some() {
        ServiceConfidence::Medium
    } else {
        ServiceConfidence::Unknown
    };
    let mut endpoints = endpoints_from_config(&spec.env, &spec.args);
    deduplicate_endpoints(&mut endpoints);
    ServiceObservation {
        classification,
        technology: TechnologyProfile {
            runtime: runtime.map(str::to_string),
            framework: framework.map(str::to_string),
            confidence,
            evidence,
        },
        endpoints,
        inspection: ServiceInspectionState::Pending,
    }
}

fn package_manifest_signals(cwd: &Path) -> Vec<String> {
    let mut signals = Vec::new();
    let package_json = cwd.join("package.json");
    if let Ok(content) = fs::read_to_string(package_json) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) {
            for section in ["dependencies", "devDependencies"] {
                if let Some(entries) = value.get(section).and_then(serde_json::Value::as_object) {
                    signals.extend(entries.keys().map(|key| format!("\"{key}\"")));
                }
            }
            if let Some(scripts) = value.get("scripts").and_then(serde_json::Value::as_object) {
                signals.extend(
                    scripts
                        .values()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_string),
                );
            }
        }
    }
    if cwd.join("go.mod").is_file() {
        signals.push("go.mod".to_string());
    }
    if let Ok(entries) = fs::read_dir(cwd) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name.ends_with(".csproj") || name.ends_with(".fsproj") {
                signals.push("aspnetcore project".to_string());
                break;
            }
        }
    }
    signals
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn endpoints_from_config(env: &BTreeMap<String, String>, args: &[String]) -> Vec<ServiceEndpoint> {
    let mut endpoints = Vec::new();
    let host = env
        .get("HOST")
        .or_else(|| env.get("BIND_HOST"))
        .cloned()
        .unwrap_or_else(|| "127.0.0.1".to_string());
    if let Some(port) = env
        .get("PORT")
        .or_else(|| env.get("SERVER_PORT"))
        .and_then(|value| value.parse::<u16>().ok())
    {
        endpoints.push(configured_endpoint(host.clone(), port, "unknown"));
    }
    if let Some(urls) = env.get("ASPNETCORE_URLS") {
        endpoints.extend(
            urls.split(';')
                .filter_map(|value| endpoint_from_url(value, "config", "configured")),
        );
    }
    for (index, arg) in args.iter().enumerate() {
        if let Some(value) = arg.strip_prefix("--port=") {
            if let Ok(port) = value.parse::<u16>() {
                endpoints.push(configured_endpoint(host.clone(), port, "unknown"));
            }
        }
        if matches!(arg.as_str(), "--port" | "-p") {
            if let Some(port) = args
                .get(index + 1)
                .and_then(|value| value.parse::<u16>().ok())
            {
                endpoints.push(configured_endpoint(host.clone(), port, "unknown"));
            }
        }
    }
    endpoints
}

fn configured_endpoint(host: String, port: u16, protocol: &str) -> ServiceEndpoint {
    ServiceEndpoint {
        bind_host: host,
        port,
        protocol: protocol.to_string(),
        pid: None,
        source: "config".to_string(),
        state: "configured".to_string(),
    }
}

pub fn inspect_listeners(pids: &[u32]) -> (Vec<ServiceEndpoint>, ServiceInspectionState) {
    if pids.is_empty() {
        return (Vec::new(), ServiceInspectionState::NotRunning);
    }
    let pid_list = pids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let output = match Command::new("lsof")
        .args([
            "-nP",
            "-a",
            "-p",
            &pid_list,
            "-iTCP",
            "-sTCP:LISTEN",
            "-Fpn",
        ])
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (Vec::new(), ServiceInspectionState::Unsupported);
        }
        Err(_) => return (Vec::new(), ServiceInspectionState::Unsupported),
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut endpoints = parse_lsof_fields(&text);
    deduplicate_endpoints(&mut endpoints);
    let state = if endpoints.is_empty() {
        ServiceInspectionState::NoListener
    } else {
        ServiceInspectionState::Listening
    };
    (endpoints, state)
}

pub fn parse_lsof_fields(text: &str) -> Vec<ServiceEndpoint> {
    let mut pid = None;
    let mut endpoints = Vec::new();
    for line in text.lines() {
        if let Some(value) = line.strip_prefix('p') {
            pid = value.parse::<u32>().ok();
        } else if let Some(value) = line.strip_prefix('n') {
            if let Some((host, port)) = parse_host_port(value) {
                endpoints.push(ServiceEndpoint {
                    bind_host: host,
                    port,
                    protocol: "tcp".to_string(),
                    pid,
                    source: "socket".to_string(),
                    state: "listening".to_string(),
                });
            }
        }
    }
    endpoints
}

fn parse_host_port(value: &str) -> Option<(String, u16)> {
    let value = value
        .trim()
        .trim_start_matches("TCP ")
        .split_whitespace()
        .next()?;
    if let Some(rest) = value.strip_prefix('[') {
        let (host, port) = rest.split_once("]:")?;
        return Some((host.to_string(), port.parse().ok()?));
    }
    let (host, port) = value.rsplit_once(':')?;
    let host = if host == "*" { "0.0.0.0" } else { host };
    Some((host.to_string(), port.parse().ok()?))
}

pub fn endpoints_from_logs<'a>(lines: impl Iterator<Item = &'a str>) -> Vec<ServiceEndpoint> {
    let mut endpoints = Vec::new();
    for line in lines {
        for word in line.split_whitespace() {
            let candidate = word.trim_matches(|character: char| {
                matches!(character, ',' | ';' | ')' | '(' | '"' | '\'')
            });
            if candidate.starts_with("http://") || candidate.starts_with("https://") {
                if let Some(endpoint) = endpoint_from_url(candidate, "log", "reported") {
                    endpoints.push(endpoint);
                }
            }
        }
    }
    deduplicate_endpoints(&mut endpoints);
    endpoints
}

fn endpoint_from_url(value: &str, source: &str, state: &str) -> Option<ServiceEndpoint> {
    let (protocol, authority_and_path) = value.split_once("://")?;
    let authority = authority_and_path.split('/').next()?.trim_end_matches('/');
    let (host, port) = if let Some(rest) = authority.strip_prefix('[') {
        let (host, port) = rest.split_once("]:")?;
        (host.to_string(), port.parse::<u16>().ok()?)
    } else if let Some((host, port)) = authority.rsplit_once(':') {
        (host.to_string(), port.parse::<u16>().ok()?)
    } else {
        (
            authority.to_string(),
            match protocol {
                "http" => 80,
                "https" => 443,
                _ => return None,
            },
        )
    };
    Some(ServiceEndpoint {
        bind_host: host,
        port,
        protocol: protocol.to_string(),
        pid: None,
        source: source.to_string(),
        state: state.to_string(),
    })
}

pub fn deduplicate_endpoints(endpoints: &mut Vec<ServiceEndpoint>) {
    let mut seen = HashSet::new();
    endpoints.retain(|endpoint| {
        seen.insert((
            endpoint.bind_host.clone(),
            endpoint.port,
            endpoint.protocol.clone(),
            endpoint.state.clone(),
        ))
    });
    endpoints.sort_by(|left, right| {
        left.bind_host
            .cmp(&right.bind_host)
            .then(left.port.cmp(&right.port))
            .then(left.state.cmp(&right.state))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(program: &str, args: &[&str], cwd: &Path) -> TaskSpec {
        TaskSpec {
            label: "service".to_string(),
            program: program.to_string(),
            args: args.iter().map(|value| value.to_string()).collect(),
            cwd: cwd.to_path_buf(),
            env: BTreeMap::new(),
            shell: false,
            auto_start: false,
            stop_timeout_ms: 3_000,
            clear_logs_on_restart: false,
        }
    }

    #[test]
    fn identifies_vite_from_manifest() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"devDependencies":{"vite":"latest"},"scripts":{"dev":"vite"}}"#,
        )
        .unwrap();
        let technology = infer_service(&spec("npm", &["run", "dev"], dir.path())).technology;
        assert_eq!(technology.runtime.as_deref(), Some("node"));
        assert_eq!(technology.framework.as_deref(), Some("vite"));
        assert_eq!(technology.confidence, ServiceConfidence::High);
    }

    #[test]
    fn parses_lsof_ipv4_wildcard_and_ipv6_listeners() {
        let endpoints = parse_lsof_fields("p42\nn*:5173\np43\nn[::1]:8000\n");
        assert_eq!(endpoints.len(), 2);
        assert_eq!(endpoints[0].bind_host, "0.0.0.0");
        assert_eq!(endpoints[0].port, 5173);
        assert_eq!(endpoints[0].pid, Some(42));
        assert_eq!(endpoints[1].bind_host, "::1");
        assert_eq!(endpoints[1].port, 8000);
    }

    #[test]
    fn parses_reported_http_url_without_marking_it_listening() {
        let endpoints =
            endpoints_from_logs(["ready at http://127.0.0.1:3000/", "other output"].into_iter());
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].protocol, "http");
        assert_eq!(endpoints[0].state, "reported");
    }
}
