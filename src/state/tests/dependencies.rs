//! dependencies domain state tests.


use super::super::*;
use crate::protocol::*;

    #[test]
    fn task_dependencies_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::open(dir.path()).unwrap();
        let dependency = store
            .create_task_dependency(TaskDependencyInput {
                node_id: "self".to_string(),
                session: "api".to_string(),
                task: "deploy".to_string(),
                depends_node_id: "self".to_string(),
                depends_session: "api".to_string(),
                depends_task: "build".to_string(),
                required_state: None,
            })
            .unwrap();
        assert_eq!(dependency.required_state, "running");

        assert!(
            store
                .create_task_dependency(TaskDependencyInput {
                    node_id: "self".to_string(),
                    session: "api".to_string(),
                    task: "build".to_string(),
                    depends_node_id: "self".to_string(),
                    depends_session: "api".to_string(),
                    depends_task: "build".to_string(),
                    required_state: None,
                })
                .is_err()
        );
        assert!(
            store
                .create_task_dependency(TaskDependencyInput {
                    node_id: "self".to_string(),
                    session: "api".to_string(),
                    task: "deploy".to_string(),
                    depends_node_id: "self".to_string(),
                    depends_session: "api".to_string(),
                    depends_task: "build".to_string(),
                    required_state: None,
                })
                .is_err()
        );
        assert!(
            store
                .create_task_dependency(TaskDependencyInput {
                    node_id: "self".to_string(),
                    session: "api".to_string(),
                    task: "deploy".to_string(),
                    depends_node_id: "self".to_string(),
                    depends_session: "api".to_string(),
                    depends_task: "build".to_string(),
                    required_state: Some("exited".to_string()),
                })
                .is_err()
        );

        let deps = store
            .dependencies_for_task("self", "api", "deploy")
            .unwrap();
        assert_eq!(deps.len(), 1);
        assert!(
            store
                .dependencies_for_task("self", "api", "build")
                .unwrap()
                .is_empty()
        );
        assert!(store.delete_task_dependency(&dependency.id).unwrap());
        assert!(store.task_dependencies().unwrap().is_empty());
    }

