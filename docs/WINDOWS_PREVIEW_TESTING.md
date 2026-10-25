# Windows preview: testing with real people

The goal of this test is to learn whether someone who has never heard of
Hyper-V, virtualization, llama.cpp, VMs, Rust or Tauri can set up Pegoles
on their own Windows PC and finish a first task **without help**. Testers
get only the short message below; everything else has to come from the app.

The preview installer is **unsigned** and **not a release** (no v0.2, no
RC). It comes from CI's `Windows installer` job on `phase/windows-0.2`
(NSIS, per machine, built from the lockfiles on a clean runner) and is
shared with invited testers directly, not published.

## The message to send testers

> Hi! Could you try an app I'm building? It takes about 15 minutes.
>
> 1. Download the attached `Pegoles-Setup-x64.exe`.
> 2. Open it and install Pegoles. Windows may say it doesn't recognize the
>    app, because it isn't signed yet: choose **More info**, then
>    **Run anyway**. Don't turn off any Windows protection. If Windows
>    refuses to open it at all, stop and tell me.
> 3. Open **Pegoles** from the Start menu.
> 4. Follow what Pegoles asks, **without asking me anything**. If it asks
>    to restart your PC, that's expected; open Pegoles again afterwards.
> 5. When it's ready, give it the first task it suggests and let it finish.
> 6. If anything fails or you get stuck, press **Save a report for help**
>    (on the screen where it failed, or in **Settings → Help**) and send me
>    the file it saves in your **Downloads** folder (`Pegoles-report-….json`).
>    Also tell me, in your own words, where you got stuck.
>
> Needs Windows 11 on a 64-bit Intel or AMD PC, 8 GB of memory (16 GB
> is better) and about 10 GB of free disk space.

## What the report contains (and never contains)

`Save a report for help` writes one JSON file (`src-tauri/src/diagnostics.rs`):

- app version, build commit, release/debug;
- the system check: Windows edition and version, processor architecture,
  memory, free disk, virtualization state (and the feature facts behind
  it), the broker service, the GPU and how Pegoles Local will run on it;
- where onboarding stopped, what setup was doing (model and computer
  image: stage, error kind), which intelligence is chosen and whether a
  cloud key exists (never the key);
- the computer's state (backend, VM, guest runtime, display);
- recent failure *codes* (which action type failed, a guest disconnect, a
  task that failed) and the last 40 lines of the guest's boot log.

It never contains screenshots, task titles or text, the agent's messages,
typed text, URLs, API keys or anything from the person's files; the home
folder appears as `~`, the user name as `<user>`, key- and email-shaped
strings are replaced.

## Reading a report

1. `system.virtualization.state`: `ready`, `needs_enable`,
   `restart_pending`, `firmware_disabled` (virtualization off in the
   BIOS/UEFI: the person needs their PC's firmware setting), `unsupported`.
   `windows.broker_installed` false on a ready PC means a broken install.
2. `onboarding.step` / `completed`: where they stopped.
3. `computer.image_setup` and `intelligence.install`: download stage and
   error (`disk_space`, `network`, `corrupted`, `other`).
4. `computer.guest_state`, `computer.viewport_state` and
   `recent_failures`: boot and guest runtime problems; `boot_log_tail` for
   the guest's own view.
5. `system.acceleration`: `vulkan` with the GPU name, or `cpu` (works, but
   each step can take a minute or more).

## Known limits of this preview

- Unsigned: SmartScreen warns; PCs with **Smart App Control** on block it
  outright (there is no safe way around that, and testers must not turn it
  off). Signing comes before any public Windows download.
- Never run on a consumer PC before this test. CI covers Windows Server
  2025 on Azure with nested virtualization; GPU (Vulkan) inference has not
  run on real hardware.
- The computer image downloads once (606 MB, then 1.7 GB on disk) from
  the immutable release `guest-image-x64-0.1`; the model (MAI-UI-2B
  Q8_0 GGUF) downloads once (2.65 GB).
