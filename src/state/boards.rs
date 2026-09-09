//! Board and board-template persistence.

use anyhow::{Context, Result, bail};
use rusqlite::params;
use uuid::Uuid;

use super::StateStore;
use super::util::*;
use crate::protocol::*;

impl StateStore {
    pub fn boards(&self) -> Result<Vec<Board>> {
        let connection = self.connection.lock().expect("state store lock");
        let mut statement = connection.prepare(
            "SELECT id, name, created_at_ms, updated_at_ms
             FROM boards
             ORDER BY name COLLATE NOCASE, created_at_ms, id",
        )?;
        let mut boards = statement
            .query_map([], |row| {
                Ok(Board {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    created_at_ms: row.get::<_, i64>(2)? as u64,
                    updated_at_ms: row.get::<_, i64>(3)? as u64,
                    cards: Vec::new(),
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut cards = connection.prepare(
            "SELECT card_id, node_id, session, task, mode, pinned
             FROM board_cards
             WHERE board_id=?1
             ORDER BY position",
        )?;
        for board in &mut boards {
            board.cards = cards
                .query_map(params![board.id], |row| {
                    Ok(BoardCard {
                        id: row.get(0)?,
                        node_id: row.get(1)?,
                        session: row.get(2)?,
                        task: row.get(3)?,
                        mode: normalize_board_card_mode(&row.get::<_, String>(4)?),
                        pinned: row.get::<_, i64>(5)? != 0,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
        }
        Ok(boards)
    }

    pub fn board(&self, id: &str) -> Result<Option<Board>> {
        Ok(self.boards()?.into_iter().find(|board| board.id == id))
    }

    pub fn create_board(&self, input: BoardInput) -> Result<Board> {
        let input = normalize_board_input(input)?;
        let now = current_timestamp_ms();
        let id = Uuid::new_v4().to_string();
        {
            let mut connection = self.connection.lock().expect("state store lock");
            let transaction = connection.transaction()?;
            transaction
                .execute(
                    "INSERT INTO boards(id, name, created_at_ms, updated_at_ms) VALUES (?1, ?2, ?3, ?4)",
                    params![id, input.name, now as i64, now as i64],
                )
                .with_context(|| format!("failed to create board '{}'", input.name))?;
            write_board_cards(&transaction, &id, &input.cards)?;
            transaction.commit()?;
        }
        self.board(&id)?
            .with_context(|| format!("board '{id}' disappeared after create"))
    }

    pub fn update_board(&self, id: &str, input: BoardInput) -> Result<Board> {
        let input = normalize_board_input(input)?;
        let now = current_timestamp_ms();
        {
            let mut connection = self.connection.lock().expect("state store lock");
            let transaction = connection.transaction()?;
            let changed = transaction
                .execute(
                    "UPDATE boards SET name=?2, updated_at_ms=?3 WHERE id=?1",
                    params![id, input.name, now as i64],
                )
                .with_context(|| format!("failed to update board '{id}'"))?;
            if changed == 0 {
                bail!("board '{id}' not found");
            }
            transaction.execute("DELETE FROM board_cards WHERE board_id=?1", params![id])?;
            write_board_cards(&transaction, id, &input.cards)?;
            transaction.commit()?;
        }
        self.board(id)?
            .with_context(|| format!("board '{id}' disappeared after update"))
    }

    pub fn delete_board(&self, id: &str) -> Result<bool> {
        let connection = self.connection.lock().expect("state store lock");
        Ok(connection.execute("DELETE FROM boards WHERE id=?1", params![id])? > 0)
    }
}

impl StateStore {
    pub fn board_templates(&self) -> Result<Vec<BoardTemplate>> {
        let connection = self.connection.lock().expect("state store lock");
        let mut statement = connection.prepare(
            "SELECT id, name, description, cards_json, created_at_ms, updated_at_ms
             FROM board_templates
             ORDER BY name COLLATE NOCASE, created_at_ms, id",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)? as u64,
                    row.get::<_, i64>(5)? as u64,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut templates = Vec::new();
        for (id, name, description, cards_json, created_at_ms, updated_at_ms) in rows {
            let cards: Vec<BoardCardInput> = serde_json::from_str(&cards_json).unwrap_or_default();
            templates.push(BoardTemplate {
                id,
                name,
                description,
                cards,
                created_at_ms,
                updated_at_ms,
            });
        }
        Ok(templates)
    }

    pub fn board_template(&self, id: &str) -> Result<Option<BoardTemplate>> {
        Ok(self
            .board_templates()?
            .into_iter()
            .find(|template| template.id == id))
    }

    pub fn create_board_template(&self, input: BoardTemplateInput) -> Result<BoardTemplate> {
        let input = normalize_board_template_input(input)?;
        let now = current_timestamp_ms();
        let id = Uuid::new_v4().to_string();
        let connection = self.connection.lock().expect("state store lock");
        connection
            .execute(
                "INSERT INTO board_templates(id, name, description, cards_json, created_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    id,
                    input.name,
                    input.description,
                    serde_json::to_string(&input.cards)?,
                    now as i64,
                    now as i64
                ],
            )
            .with_context(|| format!("failed to create board template '{}'", input.name))?;
        Ok(BoardTemplate {
            id,
            name: input.name,
            description: input.description,
            cards: input.cards,
            created_at_ms: now,
            updated_at_ms: now,
        })
    }

    pub fn delete_board_template(&self, id: &str) -> Result<bool> {
        let connection = self.connection.lock().expect("state store lock");
        Ok(connection.execute("DELETE FROM board_templates WHERE id=?1", params![id])? > 0)
    }
}

pub(super) fn normalize_board_card_mode(value: &str) -> BoardCardMode {
    match value {
        "logs" => BoardCardMode::Logs,
        "metrics" => BoardCardMode::Metrics,
        _ => BoardCardMode::Status,
    }
}

pub(super) fn normalize_board_input(mut input: BoardInput) -> Result<BoardInput> {
    input.name = input.name.trim().to_string();
    if input.name.is_empty() {
        bail!("board name cannot be empty");
    }

    for card in &mut input.cards {
        card.node_id = card.node_id.trim().to_string();
        card.session = card.session.trim().to_string();
        card.task = card.task.trim().to_string();
        if card.node_id.is_empty() || card.session.is_empty() || card.task.is_empty() {
            bail!("board cards require node_id, session, and task");
        }
    }

    Ok(input)
}

pub(super) fn write_board_cards(
    transaction: &rusqlite::Transaction<'_>,
    board_id: &str,
    cards: &[BoardCardInput],
) -> Result<()> {
    for (position, card) in cards.iter().enumerate() {
        transaction.execute(
            "INSERT INTO board_cards(board_id, position, card_id, node_id, session, task, mode, pinned)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                board_id,
                position as i64,
                Uuid::new_v4().to_string(),
                &card.node_id,
                &card.session,
                &card.task,
                card.mode.as_str(),
                card.pinned as i64
            ],
        )?;
    }
    Ok(())
}

pub(super) fn normalize_board_template_input(
    mut input: BoardTemplateInput,
) -> Result<BoardTemplateInput> {
    input.name = input.name.trim().to_string();
    if input.name.is_empty() {
        bail!("board template name cannot be empty");
    }
    if let Some(description) = &input.description {
        let description = description.trim();
        input.description = if description.is_empty() {
            None
        } else {
            Some(description.to_string())
        };
    }
    for card in &mut input.cards {
        card.node_id = card.node_id.trim().to_string();
        card.session = card.session.trim().to_string();
        card.task = card.task.trim().to_string();
        if card.node_id.is_empty() || card.session.is_empty() || card.task.is_empty() {
            bail!("board template cards require node_id, session, and task");
        }
    }
    Ok(input)
}
