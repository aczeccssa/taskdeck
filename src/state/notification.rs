//! Notification rules and records persistence.

use std::collections::HashSet;

use anyhow::{Context, Result, bail};
use rusqlite::params;
use uuid::Uuid;

use super::util::*;
use super::{StateStore, NOTIFICATION_RETENTION_LIMIT};
use crate::protocol::*;

use super::quota::normalize_quota_session;

impl StateStore {
    pub fn notification_rules(&self) -> Result<Vec<NotificationRule>> {
        let connection = self.connection.lock().expect("state store lock");
        let mut statement = connection.prepare(
            "SELECT id, name, event_types_json, scope_session, scope_task, webhook_url, enabled, created_at_ms, updated_at_ms
             FROM notification_rules
             ORDER BY name COLLATE NOCASE, created_at_ms, id",
        )?;
        let rules = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, i64>(6)? != 0,
                    row.get::<_, i64>(7)? as u64,
                    row.get::<_, i64>(8)? as u64,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut parsed = Vec::new();
        for (
            id,
            name,
            event_types_json,
            scope_session,
            scope_task,
            webhook_url,
            enabled,
            created_at_ms,
            updated_at_ms,
        ) in rules
        {
            let event_types = serde_json::from_str(&event_types_json).unwrap_or_default();
            parsed.push(NotificationRule {
                id,
                name,
                event_types,
                scope_session,
                scope_task,
                webhook_url,
                enabled,
                created_at_ms,
                updated_at_ms,
            });
        }
        Ok(parsed)
    }

    pub fn create_notification_rule(
        &self,
        input: NotificationRuleInput,
    ) -> Result<NotificationRule> {
        let input = normalize_notification_rule_input(input)?;
        let now = current_timestamp_ms();
        let id = Uuid::new_v4().to_string();
        let connection = self.connection.lock().expect("state store lock");
        connection
            .execute(
                "INSERT INTO notification_rules(id, name, event_types_json, scope_session, scope_task, webhook_url, enabled, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    id,
                    input.name,
                    serde_json::to_string(&input.event_types)?,
                    input.scope_session,
                    input.scope_task,
                    input.webhook_url,
                    input.enabled as i64,
                    now as i64,
                    now as i64
                ],
            )
            .with_context(|| format!("failed to create notification rule '{}'", input.name))?;
        Ok(NotificationRule {
            id,
            name: input.name,
            event_types: input.event_types,
            scope_session: input.scope_session,
            scope_task: input.scope_task,
            webhook_url: input.webhook_url,
            enabled: input.enabled,
            created_at_ms: now,
            updated_at_ms: now,
        })
    }

    pub fn update_notification_rule(
        &self,
        id: &str,
        input: NotificationRuleInput,
    ) -> Result<NotificationRule> {
        let input = normalize_notification_rule_input(input)?;
        let now = current_timestamp_ms();
        {
            let connection = self.connection.lock().expect("state store lock");
            let changed = connection.execute(
                "UPDATE notification_rules SET name=?2, event_types_json=?3, scope_session=?4, scope_task=?5, webhook_url=?6, enabled=?7, updated_at_ms=?8 WHERE id=?1",
                params![
                    id,
                    input.name,
                    serde_json::to_string(&input.event_types)?,
                    input.scope_session,
                    input.scope_task,
                    input.webhook_url,
                    input.enabled as i64,
                    now as i64
                ],
            )?;
            if changed == 0 {
                bail!("notification rule '{id}' not found");
            }
        }
        let mut rules = self.notification_rules()?;
        rules
            .drain(..)
            .find(|rule| rule.id == id)
            .with_context(|| format!("notification rule '{id}' disappeared after update"))
    }

    pub fn delete_notification_rule(&self, id: &str) -> Result<bool> {
        let connection = self.connection.lock().expect("state store lock");
        Ok(connection.execute("DELETE FROM notification_rules WHERE id=?1", params![id])? > 0)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_notification(
        &self,
        node_id: &str,
        rule_id: Option<&str>,
        rule_name: Option<&str>,
        event_type: &str,
        severity: &str,
        session: Option<&str>,
        task: Option<&str>,
        title: &str,
        message: &str,
        details: &serde_json::Value,
    ) -> Result<Notification> {
        let now = current_timestamp_ms();
        let connection = self.connection.lock().expect("state store lock");
        connection
            .execute(
                "INSERT INTO notifications(rule_id, rule_name, event_type, severity, node_id, session, task, title, message, details_json, read, created_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, ?11)",
                params![
                    rule_id,
                    rule_name,
                    event_type,
                    severity,
                    node_id,
                    session,
                    task,
                    title,
                    message,
                    serde_json::to_string(details)?,
                    now as i64
                ],
            )
            .with_context(|| format!("failed to insert notification '{title}'"))?;
        let id = connection.last_insert_rowid() as u64;
        let _ = connection.execute(
            "DELETE FROM notifications WHERE id IN (
                 SELECT id FROM notifications ORDER BY created_at_ms DESC, id DESC LIMIT -1 OFFSET ?1
             )",
            params![NOTIFICATION_RETENTION_LIMIT as i64],
        );
        Ok(Notification {
            id,
            node_id: node_id.to_string(),
            rule_id: rule_id.map(str::to_string),
            rule_name: rule_name.map(str::to_string),
            event_type: event_type.to_string(),
            severity: severity.to_string(),
            session: session.map(str::to_string),
            task: task.map(str::to_string),
            title: title.to_string(),
            message: message.to_string(),
            read: false,
            created_at_ms: now,
        })
    }

    pub fn notifications(&self, limit: usize) -> Result<Vec<Notification>> {
        let connection = self.connection.lock().expect("state store lock");
        let mut statement = connection.prepare(
            "SELECT id, rule_id, rule_name, event_type, severity, node_id, session, task, title, message, read, created_at_ms
             FROM notifications
             ORDER BY created_at_ms DESC, id DESC
             LIMIT ?1",
        )?;
        let notifications = statement
            .query_map(params![limit as i64], |row| {
                Ok(Notification {
                    id: row.get::<_, i64>(0)? as u64,
                    rule_id: row.get(1)?,
                    rule_name: row.get(2)?,
                    event_type: row.get(3)?,
                    severity: row.get(4)?,
                    node_id: row.get(5)?,
                    session: row.get(6)?,
                    task: row.get(7)?,
                    title: row.get(8)?,
                    message: row.get(9)?,
                    read: row.get::<_, i64>(10)? != 0,
                    created_at_ms: row.get::<_, i64>(11)? as u64,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(notifications)
    }

    pub fn unread_notification_count(&self) -> Result<u64> {
        let connection = self.connection.lock().expect("state store lock");
        let count = connection.query_row(
            "SELECT COUNT(*) FROM notifications WHERE read = 0",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(count as u64)
    }

    pub fn mark_notifications_read(&self, id: Option<u64>) -> Result<u64> {
        let connection = self.connection.lock().expect("state store lock");
        let changed = match id {
            Some(id) => connection.execute(
                "UPDATE notifications SET read = 1 WHERE id = ?1",
                params![id as i64],
            )?,
            None => connection.execute("UPDATE notifications SET read = 1 WHERE read = 0", [])?,
        };
        Ok(changed as u64)
    }

}

pub(super) fn normalize_notification_rule_input(
    mut input: NotificationRuleInput,
) -> Result<NotificationRuleInput> {
    input.name = input.name.trim().to_string();
    if input.name.is_empty() {
        bail!("notification rule name cannot be empty");
    }
    let allowed: HashSet<&str> =
        HashSet::from(["task_started", "task_exited", "task_failed", "task_stopped"]);
    let mut event_types = Vec::new();
    for event_type in &input.event_types {
        let event_type = event_type.trim();
        if !allowed.contains(event_type) {
            bail!(
                "unsupported notification event type '{event_type}' (expected one of task_started, task_exited, task_failed, task_stopped)"
            );
        }
        if !event_types
            .iter()
            .any(|existing: &String| existing == event_type)
        {
            event_types.push(event_type.to_string());
        }
    }
    if event_types.is_empty() {
        bail!("notification rules require at least one event type");
    }
    input.event_types = event_types;
    if let Some(webhook_url) = &input.webhook_url {
        let webhook_url = webhook_url.trim();
        if !webhook_url.is_empty()
            && !webhook_url.starts_with("http://")
            && !webhook_url.starts_with("https://")
        {
            bail!("webhook URL must start with http:// or https://");
        }
        input.webhook_url = if webhook_url.is_empty() {
            None
        } else {
            Some(webhook_url.to_string())
        };
    } else {
        input.webhook_url = None;
    }
    input.scope_session = normalize_quota_session(input.scope_session)?;
    input.scope_task = normalize_quota_session(input.scope_task)?;
    Ok(input)
}

