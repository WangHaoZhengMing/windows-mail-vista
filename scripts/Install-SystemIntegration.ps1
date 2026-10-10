[CmdletBinding()]
param([switch]$EnableMapi, [string]$Destination)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
if ($EnableMapi) {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'Run this script in an elevated PowerShell to enable system Simple MAPI.'
    }
}
if (-not $Destination) {
    $Destination = if ($EnableMapi) { Join-Path $env:ProgramFiles 'Windows Mail Rust' } else { Join-Path $env:LOCALAPPDATA 'Programs/Windows Mail Rust' }
}
$Destination = [IO.Path]::GetFullPath($Destination)
$buildRoot = Join-Path $projectRoot 'target/release'
$sourceExe = Join-Path $buildRoot 'WinMail.exe'
if (-not (Test-Path -LiteralPath $sourceExe)) { throw 'Build the release WinMail.exe first.' }
if ($EnableMapi) {
    foreach ($sourceDll in @((Join-Path $buildRoot 'msoe.dll'), (Join-Path $projectRoot 'target/i686-pc-windows-msvc/release/msoe.dll'))) {
        if (-not (Test-Path -LiteralPath $sourceDll)) { throw "Missing DLL: $sourceDll. Build both MAPI architectures first." }
    }
}
New-Item -ItemType Directory -Force -Path $Destination | Out-Null
Copy-Item -LiteralPath $sourceExe -Destination (Join-Path $Destination 'WinMail.exe') -Force
if ($EnableMapi) {
    Copy-Item -LiteralPath (Join-Path $buildRoot 'msoe.dll') -Destination (Join-Path $Destination 'msoe.dll') -Force
    $x86Directory = Join-Path $Destination 'x86'
    New-Item -ItemType Directory -Force -Path $x86Directory | Out-Null
    Copy-Item -LiteralPath (Join-Path $projectRoot 'target/i686-pc-windows-msvc/release/msoe.dll') -Destination (Join-Path $x86Directory 'msoe.dll') -Force
}
$installedExe = Join-Path $Destination 'WinMail.exe'
function Invoke-MailSetup([string]$Argument) {
    $process = Start-Process -FilePath $installedExe -ArgumentList $Argument -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Registration failed: $Argument ($($process.ExitCode))" }
}
Invoke-MailSetup '/register-app'
if ($EnableMapi) { Invoke-MailSetup '/register-mapi' }
$shortcutDirectory = Join-Path ([Environment]::GetFolderPath('Programs')) 'Windows Mail Rust'
New-Item -ItemType Directory -Force -Path $shortcutDirectory | Out-Null
$shortcut = (New-Object -ComObject WScript.Shell).CreateShortcut((Join-Path $shortcutDirectory 'Windows Mail Rust.lnk'))
$shortcut.TargetPath = $installedExe
$shortcut.WorkingDirectory = $Destination
$shortcut.Save()
Write-Host "Installed: $installedExe"
Write-Host 'Open Settings > Accounts > Default mail app to choose mailto and .eml defaults.'
