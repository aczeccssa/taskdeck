//! sessions domain state tests.

use std::path::Path;

use super::super::*;

    #[test]
    fn aliases_are_trimmed_unique_cleared_and_restored() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::open(dir.path()).unwrap();
        store
            .upsert_registration("api", Path::new("/tmp/api"))
            .unwrap();
        store
            .upsert_registration("web", Path::new("/tmp/web"))
            .unwrap();
        store
            .set_registration_alias("api", Some("  Backend API  "))
            .unwrap();
        assert_eq!(
            store.registrations().unwrap()[0].alias.as_deref(),
            Some("Backend API")
        );
        assert!(
            store
                .set_registration_alias("web", Some("Backend API"))
                .is_err()
        );
        store.set_registration_alias("api", Some("   ")).unwrap();
        assert_eq!(store.registrations().unwrap()[0].alias, None);
        let summaries = store.workspace_summaries().unwrap();
        assert_eq!(summaries[0].session, "api");
        assert_eq!(summaries[0].display_name, "api");
        store
            .set_registration_alias("api", Some("Backend"))
            .unwrap();
        assert_eq!(
            StateStore::open(dir.path())
                .unwrap()
                .registrations()
                .unwrap()[0]
                .alias
                .as_deref(),
            Some("Backend")
        );
    }

    #[test]
    fn registrations_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::open(dir.path()).unwrap();
        store
            .upsert_registration("api", Path::new("/tmp/api"))
            .unwrap();
        drop(store);
        let registrations = StateStore::open(dir.path())
            .unwrap()
            .registrations()
            .unwrap();
        assert_eq!(registrations.len(), 1);
        assert_eq!(registrations[0].session, "api");
        assert_eq!(registrations[0].project, Path::new("/tmp/api"));
    }

