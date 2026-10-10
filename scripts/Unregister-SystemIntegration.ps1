[CmdletBinding()]
param([Parameter(Mandatory)][string]$Destination, [switch]$DisableMapi)
$ErrorActionPreference = 'Stop'
$installationRoot = [IO.Path]::GetFullPath($Destination)
$installedExe = Join-Path $installationRoot 'WinMail.exe'
if (-not (Test-Path -LiteralPath $installedExe)) { throw 'WinMail.exe was not found in the supplied installation directory.' }
foreach ($argument in @($(if ($DisableMapi) { '/unregister-mapi' }), '/unregister-app')) {
    if (-not $argument) { continue }
    $process = Start-Process -FilePath $installedExe -ArgumentList $argument -WindowStyle Hidden -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Removal failed: $argument ($($process.ExitCode)). MAPI removal requires elevation." }
}
# Keep program files and all user mail. This operation removes only system integration.
$shortcutPath = Join-Path ([Environment]::GetFolderPath('Programs')) 'Windows Mail Rust/Windows Mail Rust.lnk'
if (Test-Path -LiteralPath $shortcutPath) { Remove-Item -LiteralPath $shortcutPath }
Write-Host 'System integration removed. Program files and mail data were retained.'
