# Changelog

## [0.3.0] - 2026-09-28

### Added

- `taskdeck.yaml` is validated against a published JSON Schema 1.0.0. The same document is embedded in the binary at build time and published with the documentation site, so editor integrations and the CLI share one source of truth. `taskdeck schema check` resolves the schema from the published document, then a valid cache, then the bundled copy. `taskdeck schema update` downloads and atomically caches it, and refuses to overwrite a newer cache with an older version. Configuration loading validates offline and never requires network access. `taskdeck init` writes `$schema` into new files; files without it stay compatible, and unknown extension fields remain allowed. Schema metadata carries its own `x-taskdeck-schema-version`, currently 1.0.0, independent of the binary version.
- One-line installers for release archives: `curl -fsSL https://raw.githubusercontent.com/aczeccssa/taskdeck/master/scripts/install.sh | bash` on Linux and macOS, and `irm https://raw.githubusercontent.com/aczeccssa/taskdeck/master/scripts/install.ps1 | iex` on Windows. Each selects the matching platform archive, verifies its SHA256 checksum before installing, stops an existing daemon, and stages the new binary so a failed download cannot leave a half-written executable. `install-local.sh` and `install-local.ps1` remain the from-source path.

### Operations and security

- The installers refuse unsupported platforms and point at the right alternative instead of guessing. `TASKDECK_VERSION`, `TASKDECK_INSTALL_DIR`, `TASKDECK_RELEASES_URL` and `TASKDECK_REPOSITORY` override the defaults, and the PowerShell installer records the install directory on the user PATH on a first install.
- `scripts/test-install-docker.sh` and `scripts/test-install-powershell.ps1` exercise both installers against a local release fixture: the happy path and rejection of a bad checksum. All release traffic stays on localhost, so the smoke tests need no network access.

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
