//! Local/remote inventory and service rows for DaemonState.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fs::{self, File, OpenOptions};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use fs2::FileExt;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
#[cfg(windows)]
use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};
#[cfg(windows)]
use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;

use super::audit::{record_audit_value, record_request_audit};
use super::client::GlobalPaths;
use super::dispatch::{
    config_write_error_response, prepare_session_config_write, reject_unavailable_session,
    require_local_execution,
};
use super::handle::handle;
use super::metrics::{
    MAX_TASK_METRIC_SAMPLES, NodeMetricsStore, TASK_METRICS_SAMPLE_INTERVAL_MS, TaskMetricsKey,
    TaskMetricsStore, TaskMetricsTarget,
};
use super::notifications::{collect_run_transitions, emit_transition_notifications};
use super::process_tree::{
    AggregatedProcessTree, ObservedProcess, aggregate_process_tree, collect_metric_targets,
    current_metric_keys, observed_processes, running_metric_targets,
};
use super::state::DaemonState;
use super::util::{ScheduleKey, current_timestamp_ms, status_label};
use crate::cluster::{LeaderCluster, RemoteRequest, spawn_worker_client};
use crate::config;
use crate::protocol::{
    AuditContext, AuditRecord, AuditSource, AuditStatus, AuditTransport, Envelope,
    NodeMetricsSample, NotificationRule, Request, Response, ScalingMetric, ScalingPolicy,
    TaskMetricsAggregate, TaskMetricsSample, TaskMetricsSnapshot, TaskProcessSnapshot, TaskStatus,
};
use crate::runtime::{SessionRuntime, Sessions};
use crate::service;
use crate::state::{NodeRole, NodeSettings, StateStore};

impl DaemonState {
    pub fn local_inventory(&self) -> Vec<crate::protocol::SessionSnapshot> {
        if !self.execution_enabled() {
            return Vec::new();
        }

        let aliases = self
            .store
            .registrations()
            .map(|registrations| {
                registrations
                    .into_iter()
                    .map(|registration| (registration.session, registration.alias))
                    .collect::<HashMap<_, _>>()
            })
            .unwrap_or_default();

        self.sessions
            .lock()
            .expect("sessions lock")
            .values_mut()
            .filter_map(|runtime| {
                runtime.set_alias(aliases.get(runtime.name()).cloned().flatten());

                runtime.snapshot(0).ok()
            })
            .collect()
    }

    pub fn node_summaries(&self) -> Vec<crate::protocol::NodeSummary> {
        let settings = self.settings.lock().expect("node settings lock").clone();

        let mut nodes = Vec::new();

        if settings.execution_enabled() {
            let mut sessions = self
                .sessions
                .lock()
                .expect("sessions lock")
                .keys()
                .cloned()
                .chain(
                    self.unavailable_sessions
                        .lock()
                        .expect("unavailable sessions lock")
                        .keys()
                        .cloned(),
                )
                .collect::<Vec<_>>();

            sessions.sort();

            sessions.dedup();

            nodes.push(crate::protocol::NodeSummary {
                id: "self".to_string(),

                name: settings.name.clone(),

                role: settings.role.as_label().to_string(),

                mode: if settings.role == NodeRole::Leader {
                    settings.leader_mode.as_label().to_string()
                } else {
                    "local_executor".to_string()
                },

                online: true,

                is_self: true,

                last_seen_ms: Some(current_timestamp_ms()),

                sessions,
            });
        }

        if settings.role == NodeRole::Leader {
            nodes.extend(self.cluster.remote_nodes());
        }

        nodes
    }
}

impl DaemonState {
    pub fn service_rows(&self, node: Option<&str>) -> Vec<serde_json::Value> {
        let inventories = match node {
            Some("self") => vec![("self".to_string(), self.local_inventory())],

            Some(node) => self
                .cluster
                .cached_inventory(node)
                .map(|inventory| vec![(node.to_string(), inventory)])
                .unwrap_or_default(),

            None => {
                let mut inventories = Vec::new();

                let settings = self.settings.lock().expect("node settings lock");

                if settings.execution_enabled() {
                    inventories.push(("self".to_string(), self.local_inventory()));
                }

                if settings.role == NodeRole::Leader {
                    for node in self.cluster.remote_nodes() {
                        if let Some(inventory) = self.cluster.cached_inventory(&node.id) {
                            inventories.push((node.id, inventory));
                        }
                    }
                }

                inventories
            }
        };

        inventories
            .into_iter()
            .flat_map(|(node_id, sessions)| {
                sessions.into_iter().flat_map(move |session| {
                    let node_id = node_id.clone();

                    session
                        .tasks
                        .into_iter()
                        .filter_map(move |(task, snapshot)| {
                            let service = snapshot.service;

                            if service.classification
                                == crate::protocol::ServiceClassification::Unknown
                                && service.endpoints.is_empty()
                            {
                                return None;
                            }

                            Some(serde_json::json!({

                                "node": node_id,

                                "session": session.name,

                                "task": task,

                                "service": service,

                            }))
                        })
                })
            })
            .collect()
    }
}
