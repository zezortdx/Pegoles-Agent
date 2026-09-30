# Contributing to Pegoles

Thanks for helping. Pegoles is a security product first: a model drives a
computer, and the code exists to make sure it can only drive the VM. Read
[docs/SECURITY.md](docs/SECURITY.md) and
[docs/PROJECT_STATE.md](docs/PROJECT_STATE.md) before a large change.

Security vulnerabilities are never reported in issues or pull requests;
see [SECURITY.md](SECURITY.md).

## Development setup

To just build and use Pegoles, run `./scripts/install.sh` (see the README):
it downloads its own pinned toolchain and needs nothing below. For
development, install the tools yourself.

Requirements: a Mac with Apple silicon (the only supported platform),
Rust (CI uses 1.97.1), Node 24 (engines allow 20 to 26), pnpm 9.15.9 (the
`packageManager` in `package.json`; `corepack enable` picks it up), and
Xcode or the Command Line Tools (Swift). Building guest images also needs
Docker and e2fsprogs (`brew install e2fsprogs`).

```bash
pnpm install --frozen-lockfile --ignore-scripts   # no dependency install scripts (as in CI)

# The Tauri shell embeds the built frontend (apps/desktop/dist) at compile
# time, so build it once before the first cargo build of a fresh clone.
pnpm --filter @pegoles/desktop build

bash scripts/check.sh          # all gates: fmt, clippy -D warnings, tests,
                               # Swift helper, frontend lint/typecheck/test/build
```

VM helper (Swift) for development. It must be re-signed with the
virtualization entitlement after every rebuild:

```bash
swift build -c release --package-path native/macos/pegoles-vm-host
bash scripts/codesign-dev.sh
```

Pegoles Local (the default on-device planner): build the pinned Python/MLX
runtime once, then install the default model (weights are never committed):

```bash
bash scripts/local-model/build-runtime.sh        # -> target/pegoles-runtime
cargo run --release -p pegoles-inference --example models -- install mai-ui-2b-6bit
```

Desktop app in development mode:

```bash
pnpm --filter @pegoles/desktop tauri dev
```

The UI can be previewed without a backend at `#/dev/shell/<scenario>`.

### Guest image loop

Images are built on the maintainer's machine, never at runtime. The fast
path patches an already-provisioned disk without booting it (paths with
spaces must be quoted):

```bash
bash scripts/build-guest-image/build-runtime.sh <out-dir>        # Docker, read-only repo mount
bash scripts/build-guest-image/patch-image.sh <provisioned.img> <out-dir>/pegoles-guest-runtime <out.img>
PEGOLES_DEBS_DIR=<debs-dir> cargo run -p pegoles-computer --example seal_image -- <out.img>
```

Unit files live in `scripts/build-guest-image/seed/units/` and feed both
the full cloud-init build and the patch path. Build container images are
pinned by digest in `scripts/build-guest-image/common.sh`.

### Hardware end-to-end test

```bash
cargo build --release -p pegoles-agent --example agent_e2e
cp native/macos/pegoles-vm-host/.build/release/pegoles-vm-host target/release/examples/
./target/release/examples/agent_e2e
```

It boots a real VM from the sealed image; destroy test computers
afterwards (disk space).

### Windows (in development)

Design: `docs/WINDOWS_ARCHITECTURE.md`. On a Windows 11 x64 PC with Rust
(MSVC), Node.js + pnpm, CMake, LLVM and the Vulkan SDK:

```bash
bash scripts/package-windows.sh        # Git Bash → target/release-artifacts/Pegoles-Setup-x64.exe
```

From a Mac, the Windows crates can be type-checked and linted with
[cargo-xwin](https://github.com/rust-cross/cargo-xwin) and the pinned
toolchain (`cargo xwin clippy --target x86_64-pc-windows-msvc -p
pegoles-desktop -p pegoles-broker -p pegoles-vm-host-windows …`); the
`windows*` CI jobs are the real check. The x64 guest image is built with
`scripts/build-guest-image/build-x64.sh` (QEMU; KVM on Linux, much slower
under TCG on a Mac). The llama.cpp worker (`workers/llama`, its own
workspace) also builds on macOS (`LIBCLANG_PATH` pointing at Homebrew's
LLVM), and `local_bench` runs GGUF models through it on the real VM.

## Invariants

A pull request that breaks one of these will not be merged:

- No host shell/file/process/URL action in `pegoles-protocol::ComputerAction`;
  `pegoles-policy::evaluate` stays an exhaustive match; every action,
  including waits, goes through Core.
- Guest data is hostile: every field is bounded, nothing panics on it, and
  the app lock is not held while waiting on the guest.
- The model API key lives in the Keychain; it is never logged, returned to
  the webview, put in model context, or passed to the local worker. (No
  key storage exists on Windows yet; cloud planners fail closed there.)
- Every planner (local or cloud) is untrusted and goes through the same
  `Planner` trait and policy path. Model output is parsed only by the
  strict parsers into typed actions.
- The local worker stays sandboxed with a cleared environment, fail
  closed: `sandbox-exec` on macOS, an AppContainer with no capabilities
  plus a job object on Windows.
- On Windows, only `PegolesVmBroker` holds virtualization privileges; it
  accepts only its typed verbs from Pegoles' own helper, and it never
  receives guest bytes.
- Only a sealed Pegoles image boots in normal flows.
- The frontend never renders model or guest text as HTML.

Every fix comes with a test that fails without it.

## Commits and pull requests

- Conventional commits: `<type>: <description>`, types `feat`, `fix`,
  `refactor`, `docs`, `test`, `chore`, `perf`, `ci`, `build`.
- Keep pull requests focused; explain what changed, why, and how you
  verified it (the template asks).
- CI must be green: the required checks are listed in
  [docs/GITHUB_RELEASE_CHECKLIST.md](docs/GITHUB_RELEASE_CHECKLIST.md).
- Dependency changes: say why, and expect the lockfile diff to be read
  (`cargo deny`, `cargo audit`, `pnpm audit` and dependency review run on
  every PR).
- Workflow changes: pin every action to a full commit SHA with the version
  in a comment, declare least-privilege `permissions`, add
  `timeout-minutes`, and never use `pull_request_target`.
- Pull requests only ever run on GitHub-hosted runners with a read-only
  token and no secrets. Self-hosted runners are never used for PRs, and
  release secrets exist only in the protected `release` environment.

By contributing you agree that your contributions are licensed under the
MIT License ([LICENSE](LICENSE)). Please follow the
[Code of Conduct](CODE_OF_CONDUCT.md).
