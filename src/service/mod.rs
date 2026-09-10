//! Managed-service inference from task specs.

mod listeners;

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

#[allow(unused_imports)]
pub(crate) use listeners::{
    deduplicate_endpoints, endpoint_from_url, endpoints_from_logs, inspect_listeners,
};

#[cfg(test)]
mod tests;
