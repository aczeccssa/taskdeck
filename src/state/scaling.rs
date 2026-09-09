//! Scaling policies and cron schedule helpers.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local, TimeZone, Utc};
use rusqlite::params;
use uuid::Uuid;

use super::StateStore;
use super::util::*;
use crate::protocol::*;

impl StateStore {
    pub fn scaling_policies(&self) -> Result<Vec<ScalingPolicy>> {
        let connection = self.connection.lock().expect("state store lock");
        let mut statement = connection.prepare(
            "SELECT id, name, enabled, watch_node_id, watch_session, watch_task, metric, scale_out_threshold, scale_in_threshold, scale_out_node_id, scale_out_session, scale_out_task, cooldown_seconds, last_action, last_action_ms, created_at_ms, updated_at_ms
             FROM scaling_policies
             ORDER BY name COLLATE NOCASE, created_at_ms, id",
        )?;
        let policies = statement
            .query_map([], |row| {
                Ok(ScalingPolicy {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    enabled: row.get::<_, i64>(2)? != 0,
                    watch_node_id: row.get(3)?,
                    watch_session: row.get(4)?,
                    watch_task: row.get(5)?,
                    metric: normalize_scaling_metric(&row.get::<_, String>(6)?),
                    scale_out_threshold: row.get(7)?,
                    scale_in_threshold: row.get(8)?,
                    scale_out_node_id: row.get(9)?,
                    scale_out_session: row.get(10)?,
                    scale_out_task: row.get(11)?,
                    cooldown_seconds: row.get::<_, i64>(12)? as u64,
                    last_action: row.get(13)?,
                    last_action_ms: row.get::<_, Option<i64>>(14)?.map(|v| v as u64),
                    created_at_ms: row.get::<_, i64>(15)? as u64,
                    updated_at_ms: row.get::<_, i64>(16)? as u64,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(policies)
    }

    pub fn create_scaling_policy(&self, input: ScalingPolicyInput) -> Result<ScalingPolicy> {
        let input = normalize_scaling_policy_input(input)?;
        let now = current_timestamp_ms();
        let id = Uuid::new_v4().to_string();
        let connection = self.connection.lock().expect("state store lock");
        connection
            .execute(
                "INSERT INTO scaling_policies(id, name, enabled, watch_node_id, watch_session, watch_task, metric, scale_out_threshold, scale_in_threshold, scale_out_node_id, scale_out_session, scale_out_task, cooldown_seconds, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    id,
                    input.name,
                    input.enabled as i64,
                    input.watch_node_id,
                    input.watch_session,
                    input.watch_task,
                    input.metric.as_str(),
                    input.scale_out_threshold,
                    input.scale_in_threshold,
                    input.scale_out_node_id,
                    input.scale_out_session,
                    input.scale_out_task,
                    input.cooldown_seconds as i64,
                    now as i64,
                    now as i64
                ],
            )
            .with_context(|| format!("failed to create scaling policy '{}'", input.name))?;
        Ok(ScalingPolicy {
            id,
            name: input.name,
            enabled: input.enabled,
            watch_node_id: input.watch_node_id,
            watch_session: input.watch_session,
            watch_task: input.watch_task,
            metric: input.metric,
            scale_out_threshold: input.scale_out_threshold,
            scale_in_threshold: input.scale_in_threshold,
            scale_out_node_id: input.scale_out_node_id,
            scale_out_session: input.scale_out_session,
            scale_out_task: input.scale_out_task,
            cooldown_seconds: input.cooldown_seconds,
            last_action: None,
            last_action_ms: None,
            created_at_ms: now,
            updated_at_ms: now,
        })
    }

    pub fn update_scaling_policy(
        &self,
        id: &str,
        input: ScalingPolicyInput,
    ) -> Result<ScalingPolicy> {
        let input = normalize_scaling_policy_input(input)?;
        let now = current_timestamp_ms();
        {
            let connection = self.connection.lock().expect("state store lock");
            let changed = connection.execute(
                "UPDATE scaling_policies SET name=?2, enabled=?3, watch_node_id=?4, watch_session=?5, watch_task=?6, metric=?7, scale_out_threshold=?8, scale_in_threshold=?9, scale_out_node_id=?10, scale_out_session=?11, scale_out_task=?12, cooldown_seconds=?13, updated_at_ms=?14 WHERE id=?1",
                params![
                    id,
                    input.name,
                    input.enabled as i64,
                    input.watch_node_id,
                    input.watch_session,
                    input.watch_task,
                    input.metric.as_str(),
                    input.scale_out_threshold,
                    input.scale_in_threshold,
                    input.scale_out_node_id,
                    input.scale_out_session,
                    input.scale_out_task,
                    input.cooldown_seconds as i64,
                    now as i64
                ],
            )?;
            if changed == 0 {
                bail!("scaling policy '{id}' not found");
            }
        }
        self.scaling_policies()?
            .into_iter()
            .find(|policy| policy.id == id)
            .with_context(|| format!("scaling policy '{id}' disappeared after update"))
    }

    pub fn delete_scaling_policy(&self, id: &str) -> Result<bool> {
        let connection = self.connection.lock().expect("state store lock");
        Ok(connection.execute("DELETE FROM scaling_policies WHERE id=?1", params![id])? > 0)
    }

    pub fn record_scaling_action(&self, id: &str, action: &str, at_ms: u64) -> Result<()> {
        let connection = self.connection.lock().expect("state store lock");
        connection.execute(
            "UPDATE scaling_policies SET last_action=?2, last_action_ms=?3 WHERE id=?1",
            params![id, action, at_ms as i64],
        )?;
        Ok(())
    }
}

pub(super) fn normalize_scaling_metric(value: &str) -> ScalingMetric {
    match value {
        "memory_bytes" => ScalingMetric::MemoryBytes,
        _ => ScalingMetric::CpuPercent,
    }
}

pub(super) fn normalize_scaling_policy_input(
    mut input: ScalingPolicyInput,
) -> Result<ScalingPolicyInput> {
    input.name = input.name.trim().to_string();
    if input.name.is_empty() {
        bail!("scaling policy name cannot be empty");
    }
    input.watch_node_id = input.watch_node_id.trim().to_string();
    input.watch_session = input.watch_session.trim().to_string();
    input.watch_task = input.watch_task.trim().to_string();
    input.scale_out_node_id = input.scale_out_node_id.trim().to_string();
    input.scale_out_session = input.scale_out_session.trim().to_string();
    input.scale_out_task = input.scale_out_task.trim().to_string();
    if input.watch_node_id.is_empty()
        || input.watch_session.is_empty()
        || input.watch_task.is_empty()
        || input.scale_out_node_id.is_empty()
        || input.scale_out_session.is_empty()
        || input.scale_out_task.is_empty()
    {
        bail!("scaling policies require a watch target and a scale-out target");
    }
    if !input.scale_out_threshold.is_finite()
        || !input.scale_in_threshold.is_finite()
        || input.scale_out_threshold <= 0.0
    {
        bail!("scaling thresholds must be positive numbers");
    }
    if input.scale_in_threshold >= input.scale_out_threshold {
        bail!("scale-in threshold must be lower than scale-out threshold");
    }
    Ok(input)
}

pub(super) fn next_after_schedule(expression: &str, after_ms: u64) -> Result<u64> {
    let fields = expression.split_whitespace().count();
    let normalized = if fields == 5 {
        format!("0 {expression}")
    } else {
        expression.to_string()
    };
    let schedule = normalized
        .parse::<cron::Schedule>()
        .context(format!("invalid cron expression '{expression}'"))?;
    let after_utc = DateTime::<Utc>::from_timestamp_millis(after_ms as i64).unwrap_or(Utc::now());
    let local_after = Local.from_utc_datetime(&after_utc.naive_utc());
    schedule
        .after(&local_after)
        .next()
        .map(|next| next.with_timezone(&Local).timestamp_millis().max(0) as u64)
        .ok_or_else(|| anyhow::anyhow!("cron expression '{expression}' has no future occurrence"))
}

pub fn validate_cron_expression(expression: &str) -> Result<()> {
    if expression.trim().is_empty() {
        bail!("schedule cannot be empty; remove it to disable scheduling")
    }
    next_after_schedule(expression.trim(), current_timestamp_ms())?;
    Ok(())
}

#[allow(dead_code)]
pub fn cron_next_after(expression: &str, after_ms: u64) -> Result<u64> {
    next_after_schedule(expression.trim(), after_ms)
}
