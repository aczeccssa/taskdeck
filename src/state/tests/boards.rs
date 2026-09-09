//! boards domain state tests.


use super::super::*;
use crate::protocol::*;

    #[test]
    fn boards_are_persisted_ordered_and_validated() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::open(dir.path()).unwrap();
        let board = store
            .create_board(crate::protocol::BoardInput {
                name: " Ops board ".to_string(),
                cards: vec![
                    crate::protocol::BoardCardInput {
                        node_id: "worker-1".to_string(),
                        session: "api".to_string(),
                        task: "migrate".to_string(),
                        mode: crate::protocol::BoardCardMode::Logs,
                        pinned: true,
                    },
                    crate::protocol::BoardCardInput {
                        node_id: "self".to_string(),
                        session: "web".to_string(),
                        task: "dev".to_string(),
                        mode: crate::protocol::BoardCardMode::Metrics,
                        pinned: false,
                    },
                ],
            })
            .unwrap();
        assert_eq!(board.name, "Ops board");
        assert_eq!(board.cards[0].mode, crate::protocol::BoardCardMode::Logs);
        assert!(board.cards[0].pinned);
        assert_eq!(board.cards[1].mode, crate::protocol::BoardCardMode::Metrics);
        assert!(!board.cards[1].pinned);

        let reopened = StateStore::open(dir.path()).unwrap();
        let restored = reopened.board(&board.id).unwrap().unwrap();
        assert_eq!(restored.cards.len(), board.cards.len());
        assert_eq!(restored.cards[0].node_id, "worker-1");
        assert_ne!(restored.cards[0].id, restored.cards[1].id);
        assert!(
            reopened
                .create_board(crate::protocol::BoardInput {
                    name: "Ops board".to_string(),
                    cards: Vec::new(),
                })
                .is_err()
        );
        assert!(
            reopened
                .create_board(crate::protocol::BoardInput {
                    name: " ".to_string(),
                    cards: Vec::new(),
                })
                .is_err()
        );
        assert!(
            reopened
                .create_board(crate::protocol::BoardInput {
                    name: "Bad card".to_string(),
                    cards: vec![crate::protocol::BoardCardInput {
                        node_id: "self".to_string(),
                        session: "web".to_string(),
                        task: " ".to_string(),
                        mode: crate::protocol::BoardCardMode::Status,
                        pinned: false,
                    }],
                })
                .is_err()
        );
        let updated = reopened
            .update_board(
                &board.id,
                crate::protocol::BoardInput {
                    name: "Updated".to_string(),
                    cards: vec![crate::protocol::BoardCardInput {
                        node_id: "self".to_string(),
                        session: "web".to_string(),
                        task: "dev".to_string(),
                        mode: crate::protocol::BoardCardMode::Status,
                        pinned: true,
                    }],
                },
            )
            .unwrap();
        assert_eq!(updated.name, "Updated");
        assert_eq!(updated.cards.len(), 1);
        assert_eq!(
            updated.cards[0].mode,
            crate::protocol::BoardCardMode::Status
        );
        assert!(updated.cards[0].pinned);
        assert!(reopened.delete_board(&board.id).unwrap());
        assert!(!reopened.delete_board(&board.id).unwrap());
        assert!(reopened.board(&board.id).unwrap().is_none());
    }

    #[test]
    fn board_templates_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::open(dir.path()).unwrap();
        let template = store
            .create_board_template(BoardTemplateInput {
                name: " Ops template ".to_string(),
                description: Some(" shared ".to_string()),
                cards: vec![BoardCardInput {
                    node_id: "self".to_string(),
                    session: "api".to_string(),
                    task: "dev".to_string(),
                    mode: BoardCardMode::Status,
                    pinned: false,
                }],
                source_board_id: None,
            })
            .unwrap();
        assert_eq!(template.name, "Ops template");
        assert_eq!(template.description.as_deref(), Some("shared"));
        let restored = StateStore::open(dir.path())
            .unwrap()
            .board_template(&template.id)
            .unwrap()
            .unwrap();
        assert_eq!(restored.cards.len(), 1);
        assert!(
            store
                .create_board_template(BoardTemplateInput {
                    name: "Ops template".to_string(),
                    description: None,
                    cards: Vec::new(),
                    source_board_id: None,
                })
                .is_err()
        );
        assert!(store.delete_board_template(&template.id).unwrap());
        assert!(store.board_template(&template.id).unwrap().is_none());
    }

