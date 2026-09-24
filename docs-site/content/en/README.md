# Taskdeck documentation

Taskdeck is a persistent control plane for project tasks. Start with the [installation guide](../../#install), then configure a worker or leader and open the Web UI on port 9837.

The site publishes a versioned guide for each Git tag. Documentation fixes on `master` also rebuild the current release path while the Cargo manifest and runtime source still match that tag; older versions stay tied to their original tags.

The 0.2.0 guide documents bounded audit retention, audit search excerpts,
explicit remote-bind opt-in, scheduled-run history completion, and offline
SQLite compaction. `CHANGELOG.md` records release notes and open verification
limits for each version.

Start at the [v0.2.0 release](https://github.com/aczeccssa/taskdeck/releases/tag/v0.2.0), read the [release changelog](https://github.com/aczeccssa/taskdeck/blob/master/CHANGELOG.md), or open the [versioned guide](https://aczeccssa.github.io/taskdeck/versions/v0.2.0/).
