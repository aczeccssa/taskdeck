# Internal cluster Compose

Private Docker network for one leader, one worker, and one middleman.

This is not a production worker deployment. The official Compose file still
ships only `leader/pure_master`. This overlay exists to exercise cluster
enrollment, remote actions, and the unified audit log.

## Modes

- `self-check` (default): middleman drives the scenario, writes a report, then
  the Compose stack is destroyed and only the report remains.
- `observer`: middleman still drives and reports, but the stack stays up so the
  leader UI and node SQLite files can be inspected.

## Run

```bash
# Self-check: run, report, then tear down.
./scripts/run-internal-cluster.sh

# Observer: keep the stack after the report is written.
./scripts/run-internal-cluster.sh --mode observer

# Explicit compose file / project name.
./scripts/run-internal-cluster.sh --mode self-check --project-name taskdeck-internal
```

The report is written to `output/internal-cluster/<timestamp>/report.json`.
In `self-check` mode the Compose project is removed afterward; only that
report directory is kept.

## Message bus

Leader, worker, and middleman share `/shared/bus`.

| Path | Writer | Meaning |
| --- | --- | --- |
| `ready/leader.json` | leader | HTTP is up and this node is a leader |
| `ready/worker.json` | worker | daemon is up, session is registered, agent is connected |
| `commands/NNN-*.json` | middleman | next action for the worker |
| `acks/NNN-*.json` | worker | local result of that command |
| `phase.json` | middleman | current scenario phase |
| `report/latest.json` | middleman | final comparison report |

Atomic writes use `*.tmp` then `mv`.
