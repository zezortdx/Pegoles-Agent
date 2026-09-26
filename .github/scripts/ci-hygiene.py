#!/usr/bin/env python3
"""Repository hygiene rules that actionlint and shellcheck do not cover.

Run from the repository root (CI: ci.yml, job "hygiene"):

    python3 .github/scripts/ci-hygiene.py

Workflows (.github/workflows/*.yml):
  - every `uses:` is a local action or pinned to a full 40-hex commit SHA
  - a top-level `permissions:` block exists
  - every job sets `timeout-minutes`
  - every actions/checkout step sets `persist-credentials: false`
  - no `pull_request_target` / `workflow_run` triggers
  - no caches (actions/cache, rust-cache, setup-node/pnpm `cache:` inputs)
  - cargo build/check/clippy/test/run/doc and `cargo deny` pass --locked;
    `pnpm install` passes --frozen-lockfile
Build scripts (scripts/):
  - no `--privileged` containers
  - container images pinned by digest (@sha256:)
  - no fixed /tmp defaults for outputs (`:-/tmp/...`)

Line-based on purpose (no YAML library on the runner): the workflows use
the plain two-space layout. Exits 1 and prints every violation.
"""

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
SHA_PIN = re.compile(r"^[\w.-]+/[\w.-]+(/[\w./-]+)?@[0-9a-f]{40}(\s+#.*)?$")
CARGO_NEEDS_LOCKED = re.compile(r"\bcargo\s+(build|check|clippy|test|run|doc|deny)\b")
# A reference to a common base image (docker.io/library/<name>:<tag>, ...).
BASE_IMAGE = re.compile(
    r"(?<![\w/$.-])(?:[\w.-]+/)*(?:rust|debian|ubuntu|alpine|python|node|busybox|fedora)"
    r":[\w][\w.-]*(@sha256:[0-9a-f]{64})?"
)


def workflow_errors(path: pathlib.Path, text: str) -> list[str]:
    errs: list[str] = []
    lines = text.splitlines()
    rel = path.relative_to(ROOT)

    if not re.search(r"^permissions:", text, re.MULTILINE):
        errs.append(f"{rel}: no top-level permissions block")
    if re.search(r"\b(pull_request_target|workflow_run)\b", text):
        errs.append(f"{rel}: pull_request_target/workflow_run trigger")

    # Jobs: two-space keys under the top-level `jobs:`.
    in_jobs, job, has_timeout = False, None, False

    def close_job() -> None:
        if job and not has_timeout:
            errs.append(f"{rel}: job '{job}' has no timeout-minutes")

    for i, line in enumerate(lines, 1):
        if re.match(r"^jobs:\s*$", line):
            in_jobs = True
            continue
        if in_jobs and re.match(r"^\S", line):
            close_job()
            in_jobs, job = False, None
        if in_jobs:
            m = re.match(r"^  ([\w-]+):\s*$", line)
            if m:
                close_job()
                job, has_timeout = m.group(1), False
            elif re.match(r"^    timeout-minutes:\s*\d+", line):
                has_timeout = True

        stripped = line.strip()
        m = re.match(r"^-?\s*uses:\s*(\S+.*)$", stripped)
        if m:
            ref = m.group(1)
            if not (ref.startswith("./") or SHA_PIN.match(ref)):
                errs.append(f"{rel}:{i}: action not pinned to a commit SHA: {ref}")
            if ref.startswith("actions/checkout@"):
                window = "\n".join(lines[i : i + 6])
                if not re.search(r"persist-credentials:\s*false", window):
                    errs.append(
                        f"{rel}:{i}: actions/checkout without persist-credentials: false"
                    )
            if re.match(r"(actions/cache|Swatinem/rust-cache)@", ref):
                errs.append(f"{rel}:{i}: cache action")
        if (
            re.match(r"^(cache|cache-dependency-path):\s*\S", stripped)
            and "false" not in stripped
        ):
            errs.append(f"{rel}:{i}: cache input")
        if (
            CARGO_NEEDS_LOCKED.search(stripped)
            and not stripped.startswith("#")
            and "--locked" not in stripped
            and "cargo install" not in stripped
        ):
            errs.append(f"{rel}:{i}: cargo without --locked: {stripped}")
        if (
            re.search(r"\bpnpm install\b", stripped)
            and not stripped.startswith("#")
            and "--frozen-lockfile" not in stripped
        ):
            errs.append(f"{rel}:{i}: pnpm install without --frozen-lockfile")
    close_job()
    return errs


def script_errors(path: pathlib.Path, text: str) -> list[str]:
    errs: list[str] = []
    rel = path.relative_to(ROOT)
    for i, line in enumerate(text.splitlines(), 1):
        if line.lstrip().startswith("#"):
            continue
        if "--privileged" in line:
            errs.append(f"{rel}:{i}: privileged container")
        if re.search(r":-/tmp/", line):
            errs.append(f"{rel}:{i}: fixed /tmp default path")
        for ref in BASE_IMAGE.finditer(line):
            if not ref.group(1):
                errs.append(
                    f"{rel}:{i}: container image without a digest: {ref.group(0)}"
                )
    return errs


def main() -> int:
    errs: list[str] = []
    for wf in sorted((ROOT / ".github/workflows").glob("*.y*ml")):
        errs += workflow_errors(wf, wf.read_text())
    for sh in sorted((ROOT / "scripts").rglob("*.sh")):
        errs += script_errors(sh, sh.read_text())
    for e in errs:
        print(f"::error::{e}")
    if errs:
        return 1
    print("hygiene: workflows and build scripts ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
