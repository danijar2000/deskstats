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
# The window and the icon by the clock.
$ui = Join-Path $dir 'deskstats-ui.exe'
$shortcut = Join-Path ([Environment]::GetFolderPath('Programs')) 'Deskstats.lnk'

function Stop-Panel {
    $open = Get-Process -Name deskstats-ui -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $ui }
    if ($open) {
        $open | Stop-Process -Force
        $open | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue
    }
}

function Get-Server {
    Get-Process -Name deskstats -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $exe }
}

function Stop-Server {
    $running = Get-Server
    if ($running) {
        $running | Stop-Process -Force
        $running | Wait-Process -Timeout 20 -ErrorAction SilentlyContinue
    }
    # A server started as administrator hides its path from an ordinary window and cannot be stopped from it.
    $session = (Get-Process -Id $PID).SessionId
    $hidden = Get-Process -Name deskstats -ErrorAction SilentlyContinue | Where-Object { $_.SessionId -eq $session -and -not $_.Path }
    if ($hidden) {
        throw 'install.ps1: deskstats is running as administrator and cannot be stopped from here; run this from PowerShell opened as administrator, or sign out and in and run it again'
    }
}

if ($Uninstall) {
    Stop-Server
    Stop-Panel
    Remove-ItemProperty -Path $run -Name deskstats -ErrorAction SilentlyContinue
    Remove-ItemProperty -Path $run -Name deskstats-tray -ErrorAction SilentlyContinue
    Remove-Item -Force $shortcut -ErrorAction SilentlyContinue
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
    $builtUi = $null
    $uiCrate = Join-Path (Split-Path -Parent $here) 'ui'
    if (Test-Path (Join-Path $uiCrate 'Cargo.toml')) {
        Push-Location $uiCrate
        try {
            cargo build --release
            if ($LASTEXITCODE -ne 0) { throw 'cargo build of the window failed' }
        } finally { Pop-Location }
        $builtUi = Join-Path $uiCrate 'target\release\deskstats-ui.exe'
    }
} else {
    $built = Join-Path $here 'deskstats.exe'
    $builtUi = Join-Path $here 'deskstats-ui.exe'
}
if (-not (Test-Path $built)) { throw "install.ps1: no program to install ($built)" }

Stop-Server
Stop-Panel
New-Item -ItemType Directory -Force -Path $dir | Out-Null
# Windows lets go of a program's file a moment after the program has stopped.
function Copy-Program {
    param([string]$From, [string]$To)
    for ($i = 0; ; $i++) {
        try { Copy-Item -Force $From $To; return } catch {
            if ($i -ge 20) { throw }
            Start-Sleep -Milliseconds 500
        }
    }
}
Copy-Program $built $exe
$panel = $builtUi -and (Test-Path $builtUi)
if ($panel) { Copy-Program $builtUi $ui }
# A fresh Windows profile may not have the key of the programs that start at sign-in yet.
if (-not (Test-Path $run)) { New-Item -Path $run -Force | Out-Null }
Set-ItemProperty -Path $run -Name deskstats -Value "`"$exe`""

# From PowerShell opened as administrator the server would run as administrator until the next sign-in, out of
# reach of an ordinary window. Explorer starts it with the rights of the desktop instead.
$admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if ($admin) {
    try { Start-Process -FilePath (Join-Path $env:WINDIR 'explorer.exe') -ArgumentList "`"$exe`"" } catch { }
    for ($i = 0; $i -lt 10 -and -not (Get-Server); $i++) { Start-Sleep -Milliseconds 500 }
}
if (-not (Get-Server)) { Start-Process -FilePath $exe -WorkingDirectory $dir }

Start-Sleep -Seconds 2
if (-not (Get-Server)) {
    throw "install.ps1: the server did not start; see $data\log.txt"
}

if ($panel) {
    Set-ItemProperty -Path $run -Name deskstats-tray -Value "`"$ui`" --tray"
    try {
        $shell = New-Object -ComObject WScript.Shell
        $link = $shell.CreateShortcut($shortcut)
        $link.TargetPath = $ui
        $link.WorkingDirectory = $dir
        $link.Description = 'Deskstats'
        $link.Save()
    } catch {
        Write-Host "Could not add Deskstats to the Start menu: $($_.Exception.Message)"
    }
    # Like the server: through Explorer when this window runs as administrator, so that it runs as the user.
    if ($admin) {
        # Started with no arguments, the program is the icon.
        try { Start-Process -FilePath (Join-Path $env:WINDIR 'explorer.exe') -ArgumentList "`"$ui`"" } catch { }
    } else {
        Start-Process -FilePath $ui -ArgumentList '--tray' -WorkingDirectory $dir
    }
}

Write-Host 'deskstats is running and starts at sign-in.'
if ($panel) { Write-Host 'Its icon is by the clock (it may sit under the ^ arrow there): a click shows the panel.' }
Write-Host ''
Write-Host 'When Windows asks whether to let deskstats through the firewall, allow it for the kind of network'
Write-Host 'this Wi-Fi is (Windows often calls a home Wi-Fi public; tick both kinds when unsure):'
Write-Host 'the phone reaches this computer over Wi-Fi on TCP 8787 and UDP 8788.'
Write-Host 'The USB cable is optional: for it, install adb (winget install Google.PlatformTools) and run this again.'
Write-Host "The log is in $data\log.txt."
