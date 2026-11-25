# Installs deskstats for this user on Windows: %LOCALAPPDATA%\deskstats\deskstats.exe, started at sign-in.
#
#   powershell -ExecutionPolicy Bypass -File install.ps1
#   powershell -ExecutionPolicy Bypass -File install.ps1 -Uninstall
param([switch]$Uninstall)

$ErrorActionPreference = 'Stop'
$dir = Join-Path $env:LOCALAPPDATA 'deskstats'
$exe = Join-Path $dir 'deskstats.exe'
$run = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$data = Join-Path $env:APPDATA 'deskstats'

function Stop-Server {
    $running = Get-Process -Name deskstats -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe }
    if ($running) {
        $running | Stop-Process -Force
        $running | Wait-Process -Timeout 20 -ErrorAction SilentlyContinue
    }
}

if ($Uninstall) {
    Stop-Server
    Remove-ItemProperty -Path $run -Name deskstats -ErrorAction SilentlyContinue
    if (Test-Path $dir) { Remove-Item -Recurse -Force $dir }
    Write-Host 'deskstats removed.'
    Write-Host "Left in place: the pairing key in $data (delete the folder to forget the phone)."
    exit 0
}

$here = Split-Path -Parent $MyInvocation.MyCommand.Path
if (Test-Path (Join-Path $here 'Cargo.toml')) {
    Push-Location $here
    try {
        cargo build --release
        if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
    } finally { Pop-Location }
    $built = Join-Path $here 'target\release\deskstats.exe'
} else {
    $built = Join-Path $here 'deskstats.exe'
}
if (-not (Test-Path $built)) { throw "install.ps1: no program to install ($built)" }

Stop-Server
New-Item -ItemType Directory -Force -Path $dir | Out-Null
Copy-Item -Force $built $exe
# A fresh Windows profile may not have the key of the programs that start at sign-in yet.
if (-not (Test-Path $run)) { New-Item -Path $run -Force | Out-Null }
Set-ItemProperty -Path $run -Name deskstats -Value "`"$exe`""
Start-Process -FilePath $exe -WorkingDirectory $dir

Start-Sleep -Seconds 2
if (-not (Get-Process -Name deskstats -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe })) {
    throw "install.ps1: the server did not start; see $data\log.txt"
}

Write-Host 'deskstats is running and starts at sign-in.'
Write-Host ''
Write-Host 'When Windows asks whether to let deskstats through the firewall, allow it on private networks:'
Write-Host 'the phone reaches this computer over Wi-Fi on TCP 8787 and UDP 8788.'
Write-Host 'For the USB cable, install adb (winget install Google.PlatformTools) and run this again.'
Write-Host "The log is in $data\log.txt."
