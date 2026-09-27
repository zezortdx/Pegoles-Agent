# Real Host Compute System smoke test on a Windows runner, through the
# product path: pegoles-vm-host.exe (the unprivileged helper, as the
# signed-in user) -> PegolesVmBroker (the installed service) -> HCS.
#
# Without -Disk the VM boots UEFI from an empty VHDX: there is no guest,
# so the firmware finds nothing to boot and waits. That still proves, on
# real Windows virtualization, the broker's HCS document, its client and
# path checks, the VM access grant, start, state, the serial pipe and
# teardown. With -Disk <x64 image.vhdx> (scripts/build-guest-image/
# build-x64.sh) the guest boots too, and the helper must reach the guest
# runtime's listener over AF_HYPERV (guest_status connected).
#
# Needs the Virtual Machine Platform feature (a runner without it reports
# a warning and skips: enabling it needs a restart).
param(
  [string]$InstallDir = (Join-Path $env:ProgramFiles 'Pegoles Agent'),
  [string]$Disk = ''
)
$ErrorActionPreference = 'Stop'
# PowerShell variable names ignore case: the computer's own disk is $vhdx
# so that it never aliases the -Disk parameter.

$vmp = Get-WindowsOptionalFeature -Online -FeatureName VirtualMachinePlatform
Write-Output "VirtualMachinePlatform: $($vmp.State)"
if ($vmp.State -ne 'Enabled') {
  Write-Output "::warning::Virtual Machine Platform is $($vmp.State) on this runner; HCS smoke skipped"
  exit 0
}

$id = [guid]::NewGuid().ToString()
$dir = Join-Path $env:LOCALAPPDATA "Pegoles\computers\$id"
New-Item -ItemType Directory -Force (Join-Path $dir 'logs') | Out-Null
$vhdx = Join-Path $dir 'disk.vhdx'
if ($Disk) {
  Copy-Item -LiteralPath $Disk -Destination $vhdx
} else {
  $script = Join-Path $env:RUNNER_TEMP 'pegoles-vhd.txt'
  "create vdisk file=`"$vhdx`" maximum=64 type=expandable" | Set-Content -Encoding ascii $script
  diskpart /s $script | Out-Null
}
if (-not (Test-Path $vhdx)) { throw 'could not create the test VHDX' }

$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = Join-Path $InstallDir 'pegoles-vm-host.exe'
$psi.RedirectStandardInput = $true
$psi.RedirectStandardOutput = $true
$psi.UseShellExecute = $false
$helper = [System.Diagnostics.Process]::Start($psi)

function Send([hashtable]$request) {
  $line = $request | ConvertTo-Json -Compress -Depth 5
  Write-Output ">> $line"
  $helper.StandardInput.WriteLine($line)
  $helper.StandardInput.Flush()
  while ($true) {
    $reply = $helper.StandardOutput.ReadLine()
    if ($null -eq $reply) { throw 'the helper exited' }
    Write-Output "<< $reply"
    $value = $reply | ConvertFrom-Json
    if ($null -ne $value.id -and $value.id -eq $request.id) { return $value }
  }
}

$params = @{
  computer_id     = $id
  disk_path       = $vhdx
  efi_vars_path   = (Join-Path $dir 'efi-vars.bin')
  machine_id_path = (Join-Path $dir 'machine-id.bin')
  serial_log_path = (Join-Path $dir 'logs\serial.log')
  vcpus           = 2
  memory_mb       = 1024
}
try {
  $r = Send @{ id = 1; command = 'version' }
  if (-not $r.ok) { throw 'version failed' }
  $r = Send @{ id = 2; command = 'create'; params = $params }
  if (-not $r.ok) { throw "create failed: $($r.error.message)" }
  $r = Send @{ id = 3; command = 'start'; computer_id = $id }
  if (-not $r.ok) { throw "start failed: $($r.error.message)" }
  Start-Sleep -Seconds 8
  $r = Send @{ id = 4; command = 'state'; computer_id = $id }
  if (-not $r.ok -or $r.state -ne 'running') { throw "expected running, got $($r.state) $($r.error.message)" }
  if ($Disk) {
    # The guest boots and its runtime starts listening; the helper keeps
    # dialing until it answers.
    $connected = $false
    for ($i = 0; $i -lt 60 -and -not $connected; $i++) {
      Start-Sleep -Seconds 3
      $r = Send @{ id = 100 + $i; command = 'guest_status'; computer_id = $id }
      $connected = [bool]$r.connected
    }
    if (-not $connected) { throw 'the guest runtime never answered over AF_HYPERV' }
    Write-Output 'guest runtime connected over AF_HYPERV'
  }
  $r = Send @{ id = 5; command = 'destroy'; computer_id = $id }
  if (-not $r.ok) { throw "destroy failed: $($r.error.message)" }
} finally {
  $helper.StandardInput.Close()
  if (-not $helper.WaitForExit(20000)) { $helper.Kill(); throw 'the helper did not exit' }
  $log = Join-Path $dir 'logs\serial.log'
  if (Test-Path $log) {
    Write-Output '--- COM1 ---'
    Get-Content $log -Tail 80
  }
}
# The lease: nothing of ours may still run once the helper is gone.
if (Get-Command hcsdiag -ErrorAction SilentlyContinue) {
  $left = hcsdiag list | Select-String $id
  if ($left) { throw "compute system $id outlived its helper" }
}
Remove-Item -Recurse -Force $dir
Write-Output 'HCS SMOKE OK'
