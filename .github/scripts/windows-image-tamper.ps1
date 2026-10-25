# Tampered image bytes must fail closed on Windows, with the product
# installer from a release build (pins enforced):
#   1. an archive with one flipped byte is refused (SHA-256 mismatch);
#   2. a truncated archive is refused;
#   3. an installed disk with one flipped byte is refused by the check that
#      runs before every boot;
#   4. the untouched installed image still verifies.
# Runs after `install_image` installed the published image into the data dir.
$ErrorActionPreference = 'Stop'
$tool = 'target\release\examples\install_image.exe'
if (-not (Test-Path $tool)) { throw "missing $tool (build the example first)" }
$work = Join-Path $env:RUNNER_TEMP 'tamper'
Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $work | Out-Null

$catalog = Get-Content 'crates\pegoles-computer\catalog\images.json' -Raw | ConvertFrom-Json
$image = $catalog.images | Where-Object { $_.architecture -eq 'amd64' }
if (-not $image) { throw 'no amd64 image in the catalog' }
$archive = Join-Path $work $image.archive.file_name
Write-Output "fetching $($image.archive.urls[0])"
curl.exe --fail --location --silent --show-error --proto '=https' -o $archive $image.archive.urls[0]
if ($LASTEXITCODE -ne 0) { throw 'download failed' }

function Expect-Refused([string]$label, [string[]]$toolArgs, [string]$needle) {
  $out = & $tool @toolArgs 2>&1 | Out-String
  if ($LASTEXITCODE -eq 0) { throw "$label was accepted:`n$out" }
  if ($out -notmatch [regex]::Escape($needle)) { throw "$label failed for another reason:`n$out" }
  Write-Output "refused as expected: $label"
}

function Flip-Byte([string]$path, [long]$offset) {
  $f = [System.IO.File]::Open($path, 'Open', 'ReadWrite')
  try {
    $f.Seek($offset, 'Begin') | Out-Null
    $b = $f.ReadByte()
    $f.Seek($offset, 'Begin') | Out-Null
    $f.WriteByte(($b -bxor 0x01))
  } finally { $f.Close() }
}

# 1. One flipped byte in the middle of the archive.
$flipped = Join-Path $work 'flipped.gz'
Copy-Item $archive $flipped
Flip-Byte $flipped ([long]((Get-Item $flipped).Length / 2))
Expect-Refused 'archive with one flipped byte' @('--from-file', $flipped, '--images-dir', (Join-Path $work 'a')) 'does not match its pinned SHA-256'

# 2. A truncated archive (the first half).
$short = Join-Path $work 'short.gz'
$in = [System.IO.File]::OpenRead($archive)
$outFile = [System.IO.File]::Create($short)
try {
  $buf = New-Object byte[] (1MB)
  $left = [long]($in.Length / 2)
  while ($left -gt 0) {
    $n = $in.Read($buf, 0, [int][Math]::Min($buf.Length, $left))
    if ($n -le 0) { break }
    $outFile.Write($buf, 0, $n)
    $left -= $n
  }
} finally { $in.Close(); $outFile.Close() }
Expect-Refused 'truncated archive' @('--from-file', $short, '--images-dir', (Join-Path $work 'b')) 'download stopped'

# 3. One flipped byte in an installed copy of the disk.
$installed = Join-Path $env:LOCALAPPDATA "Pegoles\images\$($image.id)"
if (-not (Test-Path (Join-Path $installed $image.disk.file_name))) { throw "no installed image at $installed" }
$copyRoot = Join-Path $work 'installed'
New-Item -ItemType Directory -Force $copyRoot | Out-Null
Copy-Item -Recurse $installed $copyRoot
$disk = Join-Path $copyRoot "$($image.id)\$($image.disk.file_name)"
Set-ItemProperty $disk -Name IsReadOnly -Value $false
Flip-Byte $disk ([long]64MB)
Expect-Refused 'installed disk with one flipped byte' @('--verify-only', '--images-dir', $copyRoot) 'do not match the pinned SHA-512'

# 4. The real installed image is intact.
& $tool --verify-only
if ($LASTEXITCODE -ne 0) { throw 'the installed image no longer verifies' }
Remove-Item -Recurse -Force $work
Write-Output 'tampered bytes fail closed; the installed image verifies'
