# Installs deskstats from the latest GitHub release: the server on this Windows computer and,
# when a phone is connected over USB, the app on the phone.
#
#   irm https://raw.githubusercontent.com/danijar2000/deskstats/main/setup.ps1 | iex
#
# To remove it: download this file and run   .\setup.ps1 -Uninstall
param([switch]$NoPhone, [switch]$Uninstall)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$base = 'https://github.com/danijar2000/deskstats/releases/latest/download'

if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64' -and $env:PROCESSOR_ARCHITEW6432 -ne 'AMD64') {
    throw 'setup.ps1: there is a build for 64-bit Intel/AMD Windows only; build from source on other machines'
}

# Runs adb and returns everything it printed. Windows PowerShell 5.1 turns what a program writes to its error
# stream into a terminating error while $ErrorActionPreference is 'Stop', so it is 'Continue' in here only.
function Invoke-Adb {
    param([string]$Adb, [string[]]$Arguments)
    $ErrorActionPreference = 'Continue'
    (& $Adb @Arguments 2>&1 | ForEach-Object { "$_" }) -join "`n"
}

# Shows the link to the phone app as a QR code. The server that was just installed says which squares are dark;
# they are drawn here with background colours, which every Windows console can do.
function Show-Qr {
    $link = "$base/deskstats.apk"
    $server = Join-Path $env:LOCALAPPDATA 'deskstats\deskstats.exe'
    $rows = @()
    if (Test-Path $server) { $rows = @(& $server --qr-bits $link | Where-Object { $_ -match '^[01]+$' }) }
    if ($rows.Count -gt 0) {
        $blank = '0' * $rows[0].Length
        foreach ($row in @($blank, $blank) + $rows + @($blank, $blank)) {
            foreach ($square in ("0000$row" + '0000').ToCharArray()) {
                $colour = if ($square -eq '1') { 'Black' } else { 'White' }
                Write-Host '  ' -NoNewline -BackgroundColor $colour
            }
            Write-Host ''
        }
    }
    Write-Host "  $link"
    Write-Host ''
    Write-Host "Scan the code with the phone's camera (or open the link on the phone), install the app and open it."
    Write-Host 'With the phone on the same Wi-Fi as this computer, the app finds it: tap it there and allow it here.'
}

$work = Join-Path ([IO.Path]::GetTempPath()) ("deskstats-" + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Force -Path $work | Out-Null
try {
    Write-Host '==> Downloading the server'
    $zip = Join-Path $work 'server.zip'
    try {
        Invoke-WebRequest -UseBasicParsing "$base/deskstats-server-windows-x86_64.zip" -OutFile $zip
    } catch {
        throw "setup.ps1: cannot download $base/deskstats-server-windows-x86_64.zip ($($_.Exception.Message)). If the latest release has no Windows build yet, build from source: https://github.com/danijar2000/deskstats#build-from-source"
    }
    Expand-Archive -Path $zip -DestinationPath $work -Force
    $install = Join-Path $work 'deskstats-server\install.ps1'
    if ($Uninstall) {
        & powershell -NoProfile -ExecutionPolicy Bypass -File $install -Uninstall
        return
    }
    & powershell -NoProfile -ExecutionPolicy Bypass -File $install
    if ($LASTEXITCODE -ne 0) { throw 'setup.ps1: the server was not installed' }
    if ($NoPhone) { return }

    $adb = (Get-Command adb.exe -ErrorAction SilentlyContinue).Source
    foreach ($candidate in @("$env:LOCALAPPDATA\Android\Sdk\platform-tools\adb.exe", "$env:LOCALAPPDATA\Microsoft\WinGet\Links\adb.exe", "$env:USERPROFILE\scoop\shims\adb.exe", 'C:\platform-tools\adb.exe')) {
        if (-not $adb -and (Test-Path $candidate)) { $adb = $candidate }
    }
    if (-not $adb) {
        Write-Host ''
        Write-Host 'To put the app on the phone without a cable:'
        Write-Host ''
        Show-Qr
        Write-Host ''
        Write-Host 'For the USB cable instead, install adb and run setup.ps1 again:'
        Write-Host '  winget install Google.PlatformTools'
        return
    }
    Invoke-Adb $adb @('get-state') | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Host ''
        Write-Host 'No phone over USB (USB debugging must be on, with one phone connected).'
        Write-Host 'Connect it and run setup.ps1 again, or put the app on the phone without a cable:'
        Write-Host ''
        Show-Qr
        return
    }
    Write-Host '==> Downloading the phone app'
    $apk = Join-Path $work 'deskstats.apk'
    Invoke-WebRequest -UseBasicParsing "$base/deskstats.apk" -OutFile $apk
    Write-Host '==> Installing the app on the phone'
    $out = Invoke-Adb $adb @('install', '-r', $apk)
    if ($LASTEXITCODE -ne 0) {
        Write-Host $out
        if ($out -match 'INSTALL_FAILED_UPDATE_INCOMPATIBLE|INSTALL_FAILED_VERSION_DOWNGRADE') {
            Write-Host 'The phone has a build with another signature or a newer version. Remove it'
            Write-Host '(its settings are lost) and run setup.ps1 again:'
            Write-Host "  $adb uninstall app.deskstats"
        }
        throw 'setup.ps1: the app was not installed'
    }
    Write-Host ''
    Write-Host 'Done. In a few seconds the computer opens the panel on the phone and pairs it.'
    Write-Host 'If the server was started before adb was installed, sign out and in again (or run setup.ps1 once more).'
} finally {
    Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
