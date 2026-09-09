//! workflow domain state tests.


use super::super::*;
use crate::protocol::*;

    #[test]
    fn workflow_groups_are_persisted_ordered_and_validated() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::open(dir.path()).unwrap();
        let group = store
            .create_workflow_group(crate::protocol::WorkflowGroupInput {
                name: " Release train ".to_string(),
                members: vec![
                    crate::protocol::WorkflowGroupMember {
                        node_id: "worker-1".to_string(),
                        session: "api".to_string(),
                        task: "migrate".to_string(),
                    },
                    crate::protocol::WorkflowGroupMember {
                        node_id: "self".to_string(),
                        session: "web".to_string(),
                        task: "dev".to_string(),
                    },
                ],
                graph: crate::protocol::WorkflowGraph::default(),
            })
            .unwrap();
        assert_eq!(group.name, "Release train");
        assert_eq!(group.members[0].task, "migrate");
        assert_eq!(group.members[1].node_id, "self");

        let reopened = StateStore::open(dir.path()).unwrap();
        let restored = reopened.workflow_group(&group.id).unwrap().unwrap();
        assert_eq!(restored.members, group.members);
        assert!(
            reopened
                .create_workflow_group(crate::protocol::WorkflowGroupInput {
                    name: "Release train".to_string(),
                    members: Vec::new(),
                    graph: crate::protocol::WorkflowGraph::default(),
                })
                .is_err()
        );
        assert!(
            reopened
                .create_workflow_group(crate::protocol::WorkflowGroupInput {
                    name: " ".to_string(),
                    members: Vec::new(),
                    graph: crate::protocol::WorkflowGraph::default(),
                })
                .is_err()
        );
        assert!(
            reopened
                .update_workflow_group(
                    &group.id,
                    crate::protocol::WorkflowGroupInput {
                        name: "Updated".to_string(),
                        members: vec![
                            crate::protocol::WorkflowGroupMember {
                                node_id: "self".to_string(),
                                session: "web".to_string(),
                                task: "dev".to_string(),
                            },
                            crate::protocol::WorkflowGroupMember {
                                node_id: "self".to_string(),
                                session: "web".to_string(),
                                task: "dev".to_string(),
                            },
                        ],
                        graph: crate::protocol::WorkflowGraph::default(),
                    },
                    None,
                )
                .is_err()
        );
        let updated = reopened
            .update_workflow_group(
                &group.id,
                crate::protocol::WorkflowGroupInput {
                    name: "Updated".to_string(),
                    members: group.members.iter().cloned().rev().collect(),
                    graph: crate::protocol::WorkflowGraph::default(),
                },
                None,
            )
            .unwrap();
        assert_eq!(updated.name, "Updated");
        assert_eq!(updated.members[0].session, "web");
        assert!(reopened.delete_workflow_group(&group.id).unwrap());
        assert!(!reopened.delete_workflow_group(&group.id).unwrap());
    }

    #[test]
    fn workflow_groups_persist_graph_and_record_revisions() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::open(dir.path()).unwrap();
        let graph = crate::protocol::WorkflowGraph {
            positions: vec![
                crate::protocol::WorkflowGraphNodePosition { x: 10.0, y: 20.0 },
                crate::protocol::WorkflowGraphNodePosition { x: 30.0, y: 40.0 },
            ],
            edges: vec![crate::protocol::WorkflowGraphEdge { from: 0, to: 1 }],
        };
        let group = store
            .create_workflow_group(WorkflowGroupInput {
                name: " Release pipeline ".to_string(),
                members: vec![
                    WorkflowGroupMember {
                        node_id: "self".to_string(),
                        session: "api".to_string(),
                        task: "build".to_string(),
                    },
                    WorkflowGroupMember {
                        node_id: "self".to_string(),
                        session: "api".to_string(),
                        task: "deploy".to_string(),
                    },
                ],
                graph: graph.clone(),
            })
            .unwrap();
        assert_eq!(group.name, "Release pipeline");
        assert_eq!(group.graph, graph);

        let reopened = StateStore::open(dir.path()).unwrap();
        let restored = reopened.workflow_group(&group.id).unwrap().unwrap();
        assert_eq!(restored.graph, graph);

        let revisions = reopened.workflow_revisions(&group.id).unwrap();
        assert_eq!(revisions.len(), 1);
        assert_eq!(revisions[0].revision, 1);
        assert_eq!(revisions[0].members.len(), 2);

        reopened
            .update_workflow_group(
                &group.id,
                WorkflowGroupInput {
                    name: "Release pipeline".to_string(),
                    members: restored.members.clone(),
                    graph: graph.clone(),
                },
                Some("renamed".to_string().as_str()),
            )
            .unwrap();
        let revisions = reopened.workflow_revisions(&group.id).unwrap();
        assert_eq!(revisions.len(), 2);
        assert_eq!(revisions[0].revision, 2);
        assert_eq!(revisions[0].note.as_deref(), Some("renamed"));

        // invalid edges are rejected
        let bad_edge = reopened
            .update_workflow_group(
                &group.id,
                WorkflowGroupInput {
                    name: "Release pipeline".to_string(),
                    members: restored.members.clone(),
                    graph: crate::protocol::WorkflowGraph {
                        positions: Vec::new(),
                        edges: vec![crate::protocol::WorkflowGraphEdge { from: 0, to: 5 }],
                    },
                },
                None,
            )
            .unwrap_err();
        assert!(format!("{bad_edge:#}").contains("does not exist"));

        // cycles are rejected
        let cycle = reopened
            .update_workflow_group(
                &group.id,
                WorkflowGroupInput {
                    name: "Release pipeline".to_string(),
                    members: restored.members.clone(),
                    graph: crate::protocol::WorkflowGraph {
                        positions: Vec::new(),
                        edges: vec![
                            crate::protocol::WorkflowGraphEdge { from: 0, to: 1 },
                            crate::protocol::WorkflowGraphEdge { from: 1, to: 0 },
                        ],
                    },
                },
                None,
            )
            .unwrap_err();
        assert!(format!("{cycle:#}").contains("cycles"));

        // revision retention keeps only the newest snapshots
        for index in 0..(WORKFLOW_REVISION_RETENTION_LIMIT as u64 + 2) {
            reopened
                .update_workflow_group(
                    &group.id,
                    WorkflowGroupInput {
                        name: format!("Release pipeline {index}"),
                        members: restored.members.clone(),
                        graph: graph.clone(),
                    },
                    None,
                )
                .unwrap();
        }
        let revisions = reopened.workflow_revisions(&group.id).unwrap();
        assert_eq!(revisions.len(), WORKFLOW_REVISION_RETENTION_LIMIT);
        assert_eq!(
            revisions[0].revision,
            WORKFLOW_REVISION_RETENTION_LIMIT as u64 + 4
        );
    }

