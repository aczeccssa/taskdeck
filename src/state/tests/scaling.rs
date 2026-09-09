//! scaling domain state tests.

use super::super::util::*;
use super::super::*;
use crate::protocol::*;

#[test]
fn scaling_policies_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::open(dir.path()).unwrap();
    let policy = store
        .create_scaling_policy(ScalingPolicyInput {
            name: " api autoscale ".to_string(),
            enabled: true,
            watch_node_id: "self".to_string(),
            watch_session: "api".to_string(),
            watch_task: "worker".to_string(),
            metric: ScalingMetric::CpuPercent,
            scale_out_threshold: 80.0,
            scale_in_threshold: 20.0,
            scale_out_node_id: "self".to_string(),
            scale_out_session: "api".to_string(),
            scale_out_task: "worker-replica".to_string(),
            cooldown_seconds: 60,
        })
        .unwrap();
    assert_eq!(policy.name, "api autoscale");

    assert!(
        store
            .create_scaling_policy(ScalingPolicyInput {
                name: "bad".to_string(),
                enabled: true,
                watch_node_id: "self".to_string(),
                watch_session: "api".to_string(),
                watch_task: "worker".to_string(),
                metric: ScalingMetric::CpuPercent,
                scale_out_threshold: 20.0,
                scale_in_threshold: 80.0,
                scale_out_node_id: "self".to_string(),
                scale_out_session: "api".to_string(),
                scale_out_task: "worker-replica".to_string(),
                cooldown_seconds: 60,
            })
            .is_err()
    );

    let updated = store
        .update_scaling_policy(
            &policy.id,
            ScalingPolicyInput {
                name: "api autoscale".to_string(),
                enabled: false,
                watch_node_id: "self".to_string(),
                watch_session: "api".to_string(),
                watch_task: "worker".to_string(),
                metric: ScalingMetric::MemoryBytes,
                scale_out_threshold: 1_000_000_000.0,
                scale_in_threshold: 100_000_000.0,
                scale_out_node_id: "self".to_string(),
                scale_out_session: "api".to_string(),
                scale_out_task: "worker-replica".to_string(),
                cooldown_seconds: 30,
            },
        )
        .unwrap();
    assert!(!updated.enabled);
    assert_eq!(updated.metric, ScalingMetric::MemoryBytes);

    store
        .record_scaling_action(&policy.id, "scale_out", 12345)
        .unwrap();
    let policy = store.scaling_policies().unwrap().remove(0);
    assert_eq!(policy.last_action.as_deref(), Some("scale_out"));
    assert_eq!(policy.last_action_ms, Some(12345));
    assert!(store.delete_scaling_policy(&policy.id).unwrap());
    assert!(store.scaling_policies().unwrap().is_empty());
}

#[test]
fn cron_validation_rejects_bad_expressions_and_calculates_a_future_occurrence() {
    validate_cron_expression("*/10 * * * *").unwrap();
    validate_cron_expression("*/5 * * * * *").unwrap();
    assert!(validate_cron_expression("not-a-cron").is_err());
    assert!(validate_cron_expression("").is_err());
    assert!(cron_next_after("* * * * *", current_timestamp_ms()).is_ok());
}
