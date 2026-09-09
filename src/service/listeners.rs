//! Listener inspection and endpoint parsing.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::config::TaskSpec;
use crate::protocol::{
    ServiceClassification, ServiceConfidence, ServiceEndpoint, ServiceInspectionState,
    ServiceObservation, TechnologyProfile,
};


use super::*;

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



#[cfg(windows)]

pub fn inspect_listeners(pids: &[u32]) -> (Vec<ServiceEndpoint>, ServiceInspectionState) {

    if pids.is_empty() {

        return (Vec::new(), ServiceInspectionState::NotRunning);

    }

    let output = match Command::new("powershell.exe")

        .args([

            "-NoLogo",

            "-NoProfile",

            "-NonInteractive",

            "-Command",

            "Get-NetTCPConnection -State Listen -ErrorAction Stop | ForEach-Object { '{0}`t{1}`t{2}' -f $_.OwningProcess,$_.LocalAddress,$_.LocalPort }",

        ])

        .output()

    {

        Ok(output) if output.status.success() => output,

        _ => return (Vec::new(), ServiceInspectionState::Unsupported),

    };

    let process_ids = pids

        .iter()

        .copied()

        .collect::<std::collections::HashSet<_>>();

    let text = String::from_utf8_lossy(&output.stdout);

    let mut endpoints = parse_powershell_tcp_listeners(&text)

        .into_iter()

        .filter(|endpoint| endpoint.pid.is_some_and(|pid| process_ids.contains(&pid)))

        .collect::<Vec<_>>();

    deduplicate_endpoints(&mut endpoints);

    let state = if endpoints.is_empty() {

        ServiceInspectionState::NoListener

    } else {

        ServiceInspectionState::Listening

    };

    (endpoints, state)

}



#[cfg(any(unix, test))]

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



#[cfg(any(windows, test))]

pub fn parse_powershell_tcp_listeners(text: &str) -> Vec<ServiceEndpoint> {

    text.lines()

        .filter_map(|line| {

            let mut fields = line.trim().split('\t');

            let pid = fields.next()?.parse::<u32>().ok()?;

            let bind_host = fields.next()?.trim().to_string();

            let port = fields.next()?.parse::<u16>().ok()?;

            Some(ServiceEndpoint {

                bind_host,

                port,

                protocol: "tcp".to_string(),

                pid: Some(pid),

                source: "socket".to_string(),

                state: "listening".to_string(),

            })

        })

        .collect()

}



#[cfg(any(unix, test))]

pub(crate) fn parse_host_port(value: &str) -> Option<(String, u16)> {

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



pub(crate) fn endpoint_from_url(value: &str, source: &str, state: &str) -> Option<ServiceEndpoint> {

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


