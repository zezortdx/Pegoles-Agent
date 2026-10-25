# Windows PC test plan (gates W10–W13)

For the first person who runs Pegoles on a real Windows PC. Nothing here
has been done yet (see `docs/PLATFORM_MATRIX.md`). Record every result,
screenshot the screens named below, and keep the files listed under
"Collect".

## Machines

At least two, ideally four:

| | Edition | Account | GPU |
|---|---|---|---|
| A | Windows 11 **Home** x64 | standard user (not an administrator) | any, or none |
| B | Windows 11 **Pro** x64 | administrator | NVIDIA or AMD with a current driver |
| C (optional) | Windows 11 Home | standard user | Intel integrated only |
| D (optional) | any | any | Virtual Machine Platform already on (e.g. WSL installed) |

Start each with virtualization **turned off** in Windows features
(Virtual Machine Platform unchecked), unless the row says otherwise.

## Steps

1. Download `Pegoles-Setup-x64.exe` from the release (or the CI artifact
   `Pegoles-Setup-x64-unsigned` of PR #12). Check its SHA-256 against
   the `.sha256` file. Count every click from the download to the app's
   first window, including SmartScreen and UAC.
2. Install. Expected: one UAC prompt (on a standard account: an
   administrator's password), Welcome → Next, install folder → Install,
   Finish (with "Run Pegoles Agent" ticked). Note whether
   SmartScreen appears and what it says (unsigned build: "Windows
   protected your PC"; Smart App Control, if on, blocks it: record that
   and stop on that machine).
3. First launch. Onboarding: Welcome, How it works, System check.
   - Expected on a PC with virtualization off: "Turn on virtualization"
     with an explanation, one UAC prompt, then "Restart to finish".
   - Press "Restart now". After signing in again, Pegoles must open by
     itself on the System check and show virtualization ready.
   - If the firmware has virtualization disabled, the check must say so
     and show how to enable it (nothing automatic).
4. "Set up Pegoles": model download (2.65 GB) with real progress, speed
   and time left; pause and resume once; unplug the network once and
   check the message and the retry.
   - Until the x64 computer image is published, the "Preparing your
     computer" step must stop with "Pegoles' computer isn't available for
     this PC yet" — record that it does, and that nothing else broke.
   - With a published image (or a debug build with
     `PEGOLES_IMAGE_ARCHIVE`): the computer starts; note the time.
5. Choose intelligence: Pegoles Local. The system check's acceleration
   row must match the PC (Vulkan on B, CPU or Vulkan on C).
6. First task: "Open the terminal and create a text file called
   hello.txt that says Hello from Pegoles, then show its contents."
   Record: success, number of steps, time per step, time to the first
   action.
7. Stop a second task mid-run; Reset the computer; quit Pegoles.
8. After quitting: no `pegoles-vm-host.exe`, `pegoles-llm-worker.exe`
   or compute system left (`Get-Process pegoles*`; `hcsdiag list` as an
   administrator); the `PegolesVmBroker` service stops by itself after
   about ten minutes idle.
9. Uninstall from Settings → Apps. Expected: the service is removed
   (`sc.exe query PegolesVmBroker` → 1060), `C:\Program Files\Pegoles
   Agent` is gone. The computer, image and model in
   `%LOCALAPPDATA%\Pegoles` stay (the uninstaller's "delete app data"
   box covers only the app's own settings folder); record their size.

## Security spot checks (any machine)

- From a standard user's PowerShell, try to talk to the broker pipe with
  a different program (e.g. a copy of `pegoles-vm-host.exe` in
  Downloads): the broker must refuse it.
- Replace `%LOCALAPPDATA%\Pegoles\computers\<id>` with a junction before
  pressing Start: creating the computer must fail with "Pegoles refuses
  linked folders".
- While a task runs, `Get-NetTCPConnection -OwningProcess (Get-Process
  pegoles-llm-worker).Id` must show nothing, and the worker must be in
  an AppContainer (Process Explorer: Integrity "AppContainer").
- In the guest there is no network interface other than `lo`.

## Performance to record

Per machine: model load time, step latency (median and slowest), worker
memory after load and at peak (Task Manager "Commit size"), the VM's
memory, CPU/GPU in use, and the time from "Set up Pegoles" to the first
task finished.

## Collect

`%LOCALAPPDATA%\Pegoles\computers\<id>\logs\serial.log`, the onboarding
screenshots, the SmartScreen/UAC screenshots, and the numbers above, in
an issue titled "Windows PC test: <edition>, <CPU>, <GPU>".
