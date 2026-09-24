# Changelog

## [0.2.0] - 2026-09-24

### Changed

- Bounded audit growth: keep the newest 10,000 replicated records; while a worker is offline, keep the oldest 10,000 pending records for replication and the newest 10,000 for local history. Pending records between those ranges are removed after the backlog exceeds 20,000.
- Skip successful Web `Snapshot`, `TaskLogs`, and `TaskMetrics` polling from the audit trail. Search indexes only the first 4 KiB of each request, response, and details JSON field; audit detail views still return the full stored payload.
- Avoid migration locks and integrity scans when opening an already-current SQLite database. A conflicting migration now returns a clear retryable error, and `taskdeck node integrity-check` runs the full check explicitly.
- Complete more scheduled-run history: record due runs skipped because a task is already running, and close still-running history rows when the daemon shuts down or restarts.
- Require explicit opt-in before binding a native install to a non-loopback address. The supplied Compose deployment continues to opt in intentionally; configuration files and database files use owner-only Unix permissions.

### Operations and security

- Audit retention frees SQLite pages for reuse but does not shrink an existing `state.db`. Stop the daemon, back up the state home, then run `VACUUM` followed by `PRAGMA wal_checkpoint(TRUNCATE)` to reclaim space. See the Operations guide.
- Enrollment tokens are stored as plaintext in the local `state.db`; protect the local state directory and clear tokens that are no longer needed. Node APIs redact the token.

### Follow-up

- Scheduled occurrences missed while Taskdeck is offline are still not replayed. The previously reported production history gap was not independently reproduced; this release closes the code paths verified in tests without claiming production incident parity.
- The reported task-service restart incident was not reproducible as a product defect during review. No code change is claimed for that report.
