## What and why

<!-- One or two sentences. Link the issue if there is one. -->

## How it was verified

<!-- Commands you ran and what you saw. `bash scripts/check.sh` is the
     minimum; say if you also ran the hardware E2E or a packaging build. -->

- [ ] `bash scripts/check.sh` passes locally
- [ ] New behavior has a test that fails without the change

## Security checklist

Pegoles lets an untrusted model drive a computer. If a box below does not
hold, explain why in the PR (see `docs/SECURITY.md`, `docs/THREAT_MODEL.md`).

- [ ] No new host shell/file/process/URL action; `pegoles-protocol::ComputerAction`
      is unchanged or only narrowed, and `pegoles-policy::evaluate` stays exhaustive
- [ ] Guest, model and worker output is still parsed as bounded, typed data
      (no panics, no unbounded buffers, no HTML rendering in the UI)
- [ ] No secrets, keys, tokens or personal paths in code, logs, tests or fixtures
- [ ] No change weakens the CSP, the Tauri command allow-list, the worker
      sandbox, entitlements, or hash/signature checks
- [ ] Dependency changes: lockfile diff reviewed (sources, new install scripts)
- [ ] Workflow changes: actions pinned to a full commit SHA, least-privilege
      `permissions`, no `pull_request_target`, no secrets outside `release`
