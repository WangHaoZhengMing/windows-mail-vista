[CmdletBinding()]
param([string]$WixDirectory, [string]$OutputDirectory)
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path -Parent $PSScriptRoot
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $projectRoot 'dist' }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$metadata = cargo metadata --manifest-path (Join-Path $projectRoot 'Cargo.toml') --no-deps --format-version 1 --locked | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed.' }
$version = ($metadata.packages | Where-Object name -eq 'mail_windows').version
if ($version -notmatch '^\d+\.\d+\.\d+$') { throw 'MSI requires a numeric major.minor.patch version.' }
if (-not $WixDirectory) {
    $WixDirectory = Join-Path $projectRoot 'tmp/wix-3.14.1'
    $archive = Join-Path $projectRoot 'tmp/wix-3.14.1.zip'
    if (-not (Test-Path (Join-Path $WixDirectory 'light.exe'))) {
        New-Item -ItemType Directory -Force -Path (Split-Path $archive) | Out-Null
        Invoke-WebRequest 'https://github.com/wixtoolset/wix3/releases/download/wix3141rtm/wix314-binaries.zip' -OutFile $archive
        $expected = '6AC824E1642D6F7277D0ED7EA09411A508F6116BA6FAE0AA5F2C7DAA2FF43D31'
        if ((Get-FileHash $archive -Algorithm SHA256).Hash -ne $expected) { throw 'WiX download checksum mismatch.' }
        Expand-Archive $archive -DestinationPath $WixDirectory -Force
    }
}
$payload64 = Join-Path $projectRoot 'target/x86_64-pc-windows-msvc/release'
$payload32 = Join-Path $projectRoot 'target/i686-pc-windows-msvc/release'
foreach ($file in @("$payload64/WinMail.exe", "$payload64/msoe.dll", "$payload32/msoe.dll")) {
    if (-not (Test-Path -LiteralPath $file)) { throw "Missing release binary: $file" }
}
New-Item -ItemType Directory -Force -Path $OutputDirectory | Out-Null
$object = Join-Path $OutputDirectory 'Product.wixobj'
$msi = Join-Path $OutputDirectory "Windows-Mail-Vista-$version-windows-x64.msi"
& (Join-Path $WixDirectory 'candle.exe') -nologo -arch x64 "-dVersion=$version" "-dPayload64=$payload64" "-dPayload32=$payload32" "-dProjectRoot=$projectRoot" -out $object (Join-Path $projectRoot 'packaging/windows/Product.wxs')
if ($LASTEXITCODE -ne 0) { throw 'WiX compilation failed.' }
& (Join-Path $WixDirectory 'light.exe') -nologo -out $msi $object
if ($LASTEXITCODE -ne 0) { throw 'MSI linking or validation failed.' }
$hash = (Get-FileHash $msi -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText("$msi.sha256", "$hash  $([IO.Path]::GetFileName($msi))`n", [Text.UTF8Encoding]::new($false))
Write-Output $msi
