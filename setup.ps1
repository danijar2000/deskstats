# Installs deskstats from the latest GitHub release: the server on this Windows computer and,
# when a phone is connected over USB, the app on the phone.
#
#   irm https://raw.githubusercontent.com/danijar2000/deskstats/main/setup.ps1 | iex
#
# To remove it:
#
#   & ([scriptblock]::Create((irm https://raw.githubusercontent.com/danijar2000/deskstats/main/setup.ps1))) -Uninstall
param([switch]$NoPhone, [switch]$Uninstall)

# Everything runs in a scope of its own: "irm | iex" runs this in the window of whoever typed it, and their
# settings must be as they were afterwards.
& {
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
$base = 'https://github.com/danijar2000/deskstats/releases/latest/download'

# The build is for 64-bit Intel/AMD; Windows 11 on ARM runs it as it is (it translates such programs itself).
$arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
if ($arch -ne 'AMD64' -and $arch -ne 'ARM64') {
    throw 'setup.ps1: there is a build for 64-bit Windows only (Intel/AMD, and ARM through its emulation); build from source on other machines'
}

# Runs adb and returns everything it printed. Windows PowerShell 5.1 turns what a program writes to its error
# stream into a terminating error while $ErrorActionPreference is 'Stop', so it is 'Continue' in here only.
function Invoke-Adb {
    param([string]$Adb, [string[]]$Arguments)
    $ErrorActionPreference = 'Continue'
    (& $Adb @Arguments 2>&1 | ForEach-Object { "$_" }) -join "`n"
}

# Shows the link to the phone app, to open on the phone.
function Show-Link {
    Write-Host "Open this link on the phone, install the app and open it:"
    Write-Host ''
    Write-Host "  $base/deskstats.apk"
    Write-Host ''
    Write-Host 'With the phone on the same Wi-Fi as this computer, the app finds it: tap it there and allow it here.'
}

# Checks a download against the SHA256SUMS of the release. Releases before it was published have none: then the
# download is used as it is, with a warning.
$script:sums = $null
function Test-Download {
    param([string]$File, [string]$Name)
    if ($null -eq $script:sums) {
        # Only a release without the file (404) is let through unchecked; any other failure stops here.
        try {
            $script:sums = (Invoke-WebRequest -UseBasicParsing "$base/SHA256SUMS").Content
            if ($script:sums -is [byte[]]) { $script:sums = [Text.Encoding]::ASCII.GetString($script:sums) }
        } catch {
            $code = $null
            if ($_.Exception.Response) { $code = [int]$_.Exception.Response.StatusCode }
            if ($code -ne 404) { throw "setup.ps1: cannot download the checksums of the release ($($_.Exception.Message)); try again" }
            $script:sums = ''
        }
    }
    if (-not $script:sums) {
        Write-Host "setup.ps1: this release has no checksums; $Name is not checked"
        return
    }
    $want = $null
    foreach ($line in ($script:sums -split "`n")) {
        $parts = $line.Trim() -split '\s+'
        if ($parts.Count -eq 2 -and $parts[1].TrimStart('*') -eq $Name) { $want = $parts[0].ToLowerInvariant() }
    }
    if (-not $want) { throw "setup.ps1: $Name is not in the checksums of the release" }
    $got = (Get-FileHash -Algorithm SHA256 -Path $File).Hash.ToLowerInvariant()
    if ($got -ne $want) { throw "setup.ps1: $Name does not match its checksum (got $got, want $want); not installing it" }
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
    Test-Download $zip 'deskstats-server-windows-x86_64.zip'
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
    foreach ($candidate in @("$env:LOCALAPPDATA\Android\Sdk\platform-tools\adb.exe", "$env:LOCALAPPDATA\Microsoft\WinGet\Links\adb.exe", "$env:USERPROFILE\scoop\shims\adb.exe")) {
        if (-not $adb -and (Test-Path $candidate)) { $adb = $candidate }
    }
    if (-not $adb) {
        Write-Host ''
        Write-Host 'For the USB cable, install adb and run setup.ps1 again:'
        Write-Host '  winget install Google.PlatformTools'
        Write-Host ''
        Write-Host 'To put the app on the phone without a cable:'
        Show-Link
        return
    }
    Invoke-Adb $adb @('get-state') | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Host ''
        Write-Host 'No phone over USB (USB debugging must be on, with one phone connected).'
        Write-Host 'Connect it and run setup.ps1 again, or put the app on the phone without a cable:'
        Show-Link
        return
    }
    Write-Host '==> Downloading the phone app'
    $apk = Join-Path $work 'deskstats.apk'
    Invoke-WebRequest -UseBasicParsing "$base/deskstats.apk" -OutFile $apk
    Test-Download $apk 'deskstats.apk'
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
}
