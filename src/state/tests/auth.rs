//! auth domain state tests.

use super::super::*;

#[test]
fn api_tokens_are_created_verified_and_revoked() {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::open(dir.path()).unwrap();
    let created = store.create_api_token(" ci pipeline ").unwrap();
    assert_eq!(created.token.name, "ci pipeline");
    assert!(created.secret.starts_with("tdk_"));
    assert!(created.token.token_prefix.len() < created.secret.len());
    assert!(store.verify_api_token(&created.secret).unwrap());
    assert!(!store.verify_api_token("tdk_wrong").unwrap());
    let tokens = store.api_tokens().unwrap();
    assert_eq!(tokens.len(), 1);
    assert!(tokens[0].last_used_at_ms.is_some());
    assert!(store.revoke_api_token(&created.token.id).unwrap());
    assert!(!store.verify_api_token(&created.secret).unwrap());
    assert!(store.create_api_token(" ").is_err());
}

#[test]
fn access_keys_use_argon2id_and_sessions_are_hashed() {
    let dir = tempfile::tempdir().unwrap();
    let store = StateStore::open(dir.path()).unwrap();
    let key = "correct-horse-battery-staple";
    store.set_access_key(key).unwrap();
    store.configure_auth(true).unwrap();
    assert!(verify_access_key(
        key,
        &store.auth_settings().unwrap().password_hash.unwrap()
    ));
    assert!(!verify_access_key(
        "wrong",
        &store.auth_settings().unwrap().password_hash.unwrap()
    ));
    assert!(
        !store
            .auth_settings()
            .unwrap()
            .password_hash
            .as_ref()
            .unwrap()
            .contains(key)
    );
    let token = store.create_auth_session().unwrap();
    assert!(token.len() > 32);
    assert!(store.valid_auth_session(Some(&token)));
    store.delete_auth_session(Some(&token));
    assert!(!store.valid_auth_session(Some(&token)));
    store.configure_auth(false).unwrap();
    assert!(store.auth_settings().unwrap().password_hash.is_none());
}
