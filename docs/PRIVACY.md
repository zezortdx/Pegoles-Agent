# Privacy

What Pegoles sends off your Mac, and when. This describes release 0.1 as
built from this repository; the network code paths named below are the
only ones in the app.

## Pegoles Local (the default)

Planning runs on your Mac. The model worker has no network access at all
(its macOS sandbox denies every socket, DNS included), and the app sends
no inference request anywhere. Your objectives, the screenshots of
Pegoles' computer and the model's answers stay on your Mac.

The isolated computer (the VM) has no network device, so nothing the agent
does inside it can reach the internet either.

## One-time downloads

Setting up needs the network once:

| What | From | When | Code |
|---|---|---|---|
| The local model (≈2.2 GB, MAI-UI-2B 6-bit by default) | `huggingface.co`, pinned commit | When you set up Pegoles Local | `crates/pegoles-inference/src/download.rs` |
| The Pegoles computer image (≈562 MB download, 3 GB installed) | The Pegoles GitHub release | When you set up the computer | `crates/pegoles-computer/src/image_release.rs` |

These requests send what any HTTPS download sends (your IP address, a
TLS handshake, the file being requested); nothing about you or your tasks.

Building from source (`./scripts/install.sh`) downloads its pinned build
tools and dependencies once: Rust from `static.rust-lang.org`, Node.js
from `nodejs.org`, pnpm and the frontend packages from
`registry.npmjs.org`, Rust crates from `crates.io`, the Python runtime
from GitHub (python-build-standalone) and its wheels from PyPI. The app
itself never contacts these.
Everything downloaded is checked against digests built into the app.
After setup, Pegoles Local works with the network off.

## Cloud planner (optional, off by default)

If you choose Anthropic in Settings and enter your own API key, each step
of a task sends Anthropic: the objective you typed, screenshots of
Pegoles' computer (the VM screen, never your Mac's screen), and the
conversation so far (the model's earlier actions and their results). The
request goes to `https://api.anthropic.com/v1/messages` with your key;
Anthropic's own terms and retention apply. Pegoles asks you to confirm
before switching to a cloud planner. Your key stays in the macOS
Keychain; it is never shown back in the app, logged, sent to the local
model, or placed in the model's context.

## What Pegoles never does

- No telemetry, analytics, crash reporting or update checks. There is no
  auto-updater in 0.1.
- No access to your files, your Mac's screen, clipboard, or other apps:
  Pegoles works only inside its own computer.

## What stays on disk

Under `~/Library/Application Support/Pegoles` (owner-only permissions):
the computer image and the current computer's disk, the local models,
`settings.json` (planner choice; never secrets) and the computer's serial
log. Appearance and language preferences live in `preferences/profile.json`.

`history/tasks.json` keeps up to **100 tasks**, for **30 days since the last
update**, within **512 KiB serialized JSON total**. Oldest updated tasks are
removed first when count or byte limits apply; startup and subsequent changes
apply retention on disk. With a storage failure, expired bytes may remain until
the store can be repaired; the app shows a read-only warning. Data is private
on disk, not encrypted: visible user objectives (up to 500 Unicode characters),
task IDs/status/timestamps, at most 32 content-free status transitions per task,
and at most one final agent summary result per task. A result is text (up to
4,000 Unicode characters), or a simple table (at most 8 columns × 32 data rows,
256 characters per cell and 4,000 characters across cells). These are the
agent's summaries, not files extracted from its computer.

Raw events, typed keyboard text (sensitive or ordinary), keys, screenshots,
progress notes and arbitrary diagnostics are never written to the history
file. Explicitly sensitive typing, secret-shaped typed text/objectives/results
and missing event-bus messages conservatively suppress result persistence.
Secret-shaped objectives become `Private task`. The secret-pattern tripwire
is a heuristic: ordinary objectives and allowed summaries can still contain
private information. All on-disk task content should be treated as private.
The live event list remains session-only and bounded to 200 events.

Open tasks (including pending and waiting for approval) recover as cancelled;
restart never resumes work or grants permission. Invalid, oversized, symlinked
or unreadable history is preserved and not overwritten; disk failures show
`History is read-only` in the sidebar. New tasks can still run in memory, but
new history changes are not saved until local repair and restart. Quit before
moving/removing the damaged `history` directory to repair it; preserve a copy
if its contents matter. There is no model/webview-selected file path or file
export API.
Uninstalling: quit Pegoles, delete the app, and delete that folder; if you
stored an Anthropic key, remove the `dev.pegoles.agent` item from
Keychain Access.
