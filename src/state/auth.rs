//! Auth settings, API tokens and access-key hashing.


use anyhow::{Context, Result, bail};
use argon2::{
    Argon2,
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::util::*;
use super::{StateStore, AUTH_SESSION_TTL_SECONDS};
use crate::protocol::*;

impl StateStore {
    pub fn create_api_token(&self, name: &str) -> Result<ApiTokenCreated> {
        let name = name.trim();
        if name.is_empty() {
            bail!("token name cannot be empty");
        }
        let secret = format!("tdk_{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let now = current_timestamp_ms();
        let id = Uuid::new_v4().to_string();
        let token_prefix: String = secret.chars().take(12).collect();
        let connection = self.connection.lock().expect("state store lock");
        connection
            .execute(
                "INSERT INTO api_tokens(id, name, token_hash, token_prefix, created_at_ms) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, name, sha256_hex(secret.as_bytes()), token_prefix, now as i64],
            )
            .with_context(|| format!("failed to create API token '{name}'"))?;
        Ok(ApiTokenCreated {
            token: ApiToken {
                id,
                name: name.to_string(),
                token_prefix,
                created_at_ms: now,
                last_used_at_ms: None,
                revoked: false,
            },
            secret,
        })
    }

    pub fn api_tokens(&self) -> Result<Vec<ApiToken>> {
        let connection = self.connection.lock().expect("state store lock");
        let mut statement = connection.prepare(
            "SELECT id, name, token_prefix, created_at_ms, last_used_at_ms, revoked
             FROM api_tokens
             ORDER BY created_at_ms DESC, id",
        )?;
        let tokens = statement
            .query_map([], |row| {
                Ok(ApiToken {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    token_prefix: row.get(2)?,
                    created_at_ms: row.get::<_, i64>(3)? as u64,
                    last_used_at_ms: row.get::<_, Option<i64>>(4)?.map(|v| v as u64),
                    revoked: row.get::<_, i64>(5)? != 0,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(tokens)
    }

    pub fn revoke_api_token(&self, id: &str) -> Result<bool> {
        let connection = self.connection.lock().expect("state store lock");
        Ok(connection.execute(
            "UPDATE api_tokens SET revoked = 1 WHERE id = ?1",
            params![id],
        )? > 0)
    }

    pub fn verify_api_token(&self, secret: &str) -> Result<bool> {
        let connection = self.connection.lock().expect("state store lock");
        let revoked = connection
            .query_row(
                "SELECT revoked FROM api_tokens WHERE token_hash = ?1",
                params![sha256_hex(secret.as_bytes())],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        match revoked {
            Some(0) => {
                let _ = connection.execute(
                    "UPDATE api_tokens SET last_used_at_ms = ?1 WHERE token_hash = ?2",
                    params![current_timestamp_ms() as i64, sha256_hex(secret.as_bytes())],
                );
                Ok(true)
            }
            _ => Ok(false),
        }
    }

}

impl StateStore {
    pub fn auth_settings(&self) -> Result<AuthSettings> {
        let connection = self.connection.lock().expect("state store lock");
        read_auth_settings(&connection)
    }

    pub fn apply_auth_environment(&self) -> Result<AuthSettings> {
        let connection = self.connection.lock().expect("state store lock");
        let mut settings = read_auth_settings(&connection)?;
        if let Ok(value) = std::env::var("TASKDECK_AUTH_ENABLED") {
            settings.enabled = matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            );
        }
        if settings.enabled && settings.password_hash.is_none() {
            let default = std::env::var("TASKDECK_ACCESS_KEY").map_err(|_| {
                anyhow::anyhow!("auth is enabled but no TASKDECK_ACCESS_KEY is configured")
            })?;
            settings.password_hash = Some(hash_access_key(&default)?);
        }
        write_auth_settings(&connection, &settings)?;
        Ok(settings)
    }

    pub fn configure_auth(&self, enabled: bool) -> Result<AuthSettings> {
        let connection = self.connection.lock().expect("state store lock");
        let mut settings = read_auth_settings(&connection)?;
        if !enabled {
            settings.password_hash = None;
        }
        settings.enabled = enabled;
        write_auth_settings(&connection, &settings)?;
        Ok(settings)
    }

    pub fn set_access_key(&self, key: &str) -> Result<()> {
        if key.trim().is_empty() {
            bail!("access key cannot be empty");
        }
        let connection = self.connection.lock().expect("state store lock");
        let mut settings = read_auth_settings(&connection)?;
        settings.password_hash = Some(hash_access_key(key)?);
        write_auth_settings(&connection, &settings)?;
        Ok(())
    }

    pub fn create_auth_session(&self) -> Result<String> {
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        let now = current_timestamp_ms();
        let expires = now + AUTH_SESSION_TTL_SECONDS * 1000;
        self.purge_expired_auth_sessions(now)?;
        let connection = self.connection.lock().expect("state store lock");
        connection.execute(
            "INSERT INTO auth_sessions(token_hash, created_at_ms, expires_at_ms, last_seen_at_ms) VALUES (?1, ?2, ?3, ?3)",
            params![sha256_hex(token.as_bytes()), now as i64, expires as i64],
        )?;
        Ok(token)
    }

    pub fn valid_auth_session(&self, token: Option<&str>) -> bool {
        let Some(token) = token else { return false };
        let now = current_timestamp_ms();
        let connection = match self.connection.try_lock() {
            Ok(connection) => connection,
            Err(_) => return false,
        };
        let _ = connection.execute(
            "DELETE FROM auth_sessions WHERE expires_at_ms <= ?1",
            params![now as i64],
        );
        connection.execute(
            "UPDATE auth_sessions SET last_seen_at_ms=?2 WHERE token_hash=?1 AND expires_at_ms > ?2",
            params![sha256_hex(token.as_bytes()), now as i64],
        ).is_ok_and(|count| count == 1)
    }

    pub fn delete_auth_session(&self, token: Option<&str>) {
        if let Some(token) = token {
            let _ = self.connection.lock().expect("state store lock").execute(
                "DELETE FROM auth_sessions WHERE token_hash=?1",
                params![sha256_hex(token.as_bytes())],
            );
        }
    }

    pub fn purge_expired_auth_sessions(&self, now_ms: u64) -> Result<()> {
        let connection = self.connection.lock().expect("state store lock");
        connection.execute(
            "DELETE FROM auth_sessions WHERE expires_at_ms <= ?1",
            params![now_ms as i64],
        )?;
        Ok(())
    }

    pub fn verify_access_key(&self, candidate: &str) -> Result<bool> {
        let settings = self.auth_settings()?;
        let Some(hash) = settings.password_hash else {
            return Ok(false);
        };
        Ok(verify_access_key(candidate, &hash))
    }

}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthSettings {
    pub enabled: bool,
    #[serde(skip_serializing)]
    pub password_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicAuthStatus {
    pub enabled: bool,
    pub configured: bool,
}

impl AuthSettings {
    pub fn public(&self) -> PublicAuthStatus {
        PublicAuthStatus {
            enabled: self.enabled,
            configured: self.password_hash.is_some(),
        }
    }
}
pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut out, b| {
            use std::fmt::Write as _;
            let _ = write!(out, "{b:02x}");
            out
        })
}

pub fn hash_access_key(key: &str) -> Result<String> {
    let uuid = Uuid::new_v4();
    let salt = SaltString::encode_b64(uuid.as_bytes())
        .map_err(|error| anyhow::anyhow!("failed to create password salt: {error}"))?;
    Argon2::default()
        .hash_password(key.as_bytes(), &salt)
        .map(|value| value.to_string())
        .map_err(|error| anyhow::anyhow!("failed to hash access key: {error}"))
}

pub fn verify_access_key(candidate: &str, hash: &str) -> bool {
    PasswordHash::new(hash).ok().is_some_and(|parsed| {
        Argon2::default()
            .verify_password(candidate.as_bytes(), &parsed)
            .is_ok()
    })
}

pub(super) fn read_auth_settings(connection: &Connection) -> Result<AuthSettings> {
    let (enabled, password_hash) = connection.query_row(
        "SELECT enabled,password_hash FROM auth_settings WHERE id=1",
        [],
        |row| Ok((row.get::<_, i64>(0)? != 0, row.get::<_, Option<String>>(1)?)),
    )?;
    Ok(AuthSettings {
        enabled,
        password_hash,
    })
}

pub(super) fn write_auth_settings(connection: &Connection, settings: &AuthSettings) -> Result<()> {
    connection.execute(
        "UPDATE auth_settings SET enabled=?1,password_hash=?2,updated_at_ms=?3 WHERE id=1",
        params![
            i64::from(settings.enabled),
            settings.password_hash,
            current_timestamp_ms() as i64
        ],
    )?;
    Ok(())
}

