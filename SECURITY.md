# Security policy

Pegoles lets an untrusted model operate a computer. Its whole point is
that a fully prompt-injected model, a hostile page or file inside the VM,
or a compromised guest OS cannot reach your Mac. We take reports that
break that promise seriously.

## Supported versions

| Version | Supported |
|---|---|
| 0.1.x (pre-release) | Yes: fixes land on `main` and in the next 0.1.x build |
| anything older | No |

Only builds published on this repository's GitHub Releases page (Developer
ID signed, notarized, with `SHA256SUMS` and a build provenance attestation)
are supported. Verify a download with `gh attestation verify <dmg> --repo
<this repository>` before reporting a problem with a binary obtained
elsewhere.

## Reporting a vulnerability

**Do not open a public issue, discussion or pull request.** Use GitHub
private vulnerability reporting: open this repository's **Security** tab
and choose **Report a vulnerability**. The report is visible only to the
maintainers, and we can collaborate on a fix and an advisory there.

Please include:

- the affected version (tag or DMG name) and macOS version / Mac model;
- the boundary you crossed (see below) and the impact on the host;
- a minimal reproduction: planner output, guest-side program, file or web
  content, or IPC call sequence, and the exact steps;
- whether it needs a non-default setting, a debug build, or local access;
- any proof-of-concept code (attach it to the private report, not to a
  public place).

What to expect:

- acknowledgement within 7 days, and an initial assessment within 14;
- we keep you informed while we fix it, and credit you in the advisory
  unless you prefer otherwise;
- **coordinated disclosure within 90 days** of the report, or earlier once
  a fixed release is published. If a fix needs longer, we will agree on a
  date with you. If a vulnerability is being exploited in the wild, we may
  publish sooner.

We do not run a bug bounty.

## Security boundaries

The full model, with every boundary and its enforcement, is in
[docs/SECURITY.md](docs/SECURITY.md); threats and accepted residual risks
are in [docs/THREAT_MODEL.md](docs/THREAT_MODEL.md). In short:

- **Model to host.** A planner (local or cloud) can only propose typed
  actions: observe, pointer, keyboard, wait. There is no host shell, file,
  process or URL action. Every action passes a deterministic, exhaustive
  policy and then Core's executor.
- **VM isolation.** The VM has no network device, no shared folders, no
  clipboard and no host input devices. Reset restores a sealed image.
- **Guest to host.** Everything from the guest is hostile, bounded data;
  only the guest runtime (reserved source port) can speak for the guest.
- **Local model worker.** Runs sandboxed on the host with no network and a
  cleared environment; its output is parsed like any other model output.
- **Webview to backend.** A fixed Tauri command set, a strict CSP, no
  model or guest text rendered as HTML.
- **Secrets.** The optional Anthropic API key lives in the macOS Keychain
  and never reaches the webview, logs, model context, guest or worker.
- **Supply chain and release.** Lockfiles with hashes, pinned models and
  guest image, SHA-pinned CI actions, signed and notarized releases.

Anything that crosses one of these boundaries is a vulnerability we want
to hear about.

## Not a vulnerability

These are known properties of the design, not bugs (reports that show one
of them leading to a boundary crossing are welcome):

- Anything a planner can do **inside the offline VM**: typing, clicking,
  deleting files in the guest, being prompt-injected by on-screen text.
  The VM is disposable and has no network; that is the design.
- A compromised guest killing or starving its own guest runtime (denial of
  service of the agent, not host access).
- **Hypervisor escapes** in Apple's Virtualization.framework are Apple's to
  fix (report them to Apple), but please tell us too so we can assess and
  mitigate.
- Self-XSS or other attacks that require the user to open the webview's
  developer tools and paste code.
- Attacks that require already running code as the same user on the host
  (such a process can already read the user's files and Keychain items it
  has access to).
- Behavior of debug builds or of the Design Lab / developer-only commands
  that release builds do not compile in.
- Findings from automated scanners without a demonstrated impact.
