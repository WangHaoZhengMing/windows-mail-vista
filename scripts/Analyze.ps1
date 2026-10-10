param(
    [string]$Source = 'C:\Program Files\Windows Mail',
    [string]$Llvm = 'C:\Program Files\LLVM\bin',
    [string]$Output = (Join-Path $PSScriptRoot '..\analysis')
)
$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Path $Output -Force | Out-Null
$manifest = @()
foreach ($name in @('WinMail.exe', 'msoe.dll')) {
    $path = Join-Path $Source $name
    $bytes = [IO.File]::ReadAllBytes($path)
    $pe = [BitConverter]::ToInt32($bytes, 0x3c)
    $optional = $pe + 24
    $imageBase = [BitConverter]::ToUInt64($bytes, $optional + 24)
    $sections = @()
    $sectionStart = $optional + [BitConverter]::ToUInt16($bytes, $pe + 20)
    for ($i = 0; $i -lt [BitConverter]::ToUInt16($bytes, $pe + 6); $i++) {
        $s = $sectionStart + 40 * $i
        $sections += [pscustomobject]@{
            Name = [Text.Encoding]::ASCII.GetString($bytes, $s, 8).Trim([char]0)
            Rva = [BitConverter]::ToUInt32($bytes, $s + 12)
            Size = [BitConverter]::ToUInt32($bytes, $s + 16)
            Offset = [BitConverter]::ToUInt32($bytes, $s + 20)
        }
    }
    $strings = @()
    foreach ($encoding in @('ASCII', 'UTF16')) {
        $step = if ($encoding -eq 'ASCII') { 1 } else { 2 }
        for ($i = 0; $i -lt $bytes.Length; $i += $step) {
            $start = $i
            $chars = [Text.StringBuilder]::new()
            while ($i -lt $bytes.Length -and $bytes[$i] -ge 32 -and $bytes[$i] -le 126 -and ($step -eq 1 -or ($i + 1 -lt $bytes.Length -and $bytes[$i + 1] -eq 0))) {
                [void]$chars.Append([char]$bytes[$i])
                $i += $step
            }
            if ($chars.Length -ge 4) {
                $section = $sections | Where-Object { $start -ge $_.Offset -and $start -lt $_.Offset + $_.Size } | Select-Object -First 1
                $va = if ($section) { '0x{0:x}' -f ($imageBase + $section.Rva + $start - $section.Offset) } else { '-' }
                $strings += [pscustomobject]@{ VA = $va; FileOffset = ('0x{0:x}' -f $start); Encoding = $encoding; Text = $chars.ToString() }
            }
        }
    }
    $strings | Export-Csv -LiteralPath (Join-Path $Output "$name.strings.csv") -NoTypeInformation -Encoding utf8
    & (Join-Path $Llvm 'llvm-readobj.exe') --file-headers --sections --coff-imports --coff-exports --unwind $path | Set-Content -LiteralPath (Join-Path $Output "$name.pe.txt") -Encoding utf8
    if ($LASTEXITCODE -ne 0) { throw 'llvm-readobj failed' }
    & (Join-Path $Llvm 'llvm-objdump.exe') -d --print-imm-hex --x86-asm-syntax=intel $path | Set-Content -LiteralPath (Join-Path $Output $(if ($name -eq 'WinMail.exe') { 'WinMail.disasm.txt' } else { 'msoe.disasm.txt' })) -Encoding utf8
    if ($LASTEXITCODE -ne 0) { throw 'llvm-objdump failed' }
    if ($name -eq 'WinMail.exe') {
        $labels = @{}
        foreach ($s in $strings) { if ($s.VA -ne '-') { $labels[$s.VA.ToLower()] = $s.Text } }
        $iat = 0L
        $dll = ''
        foreach ($line in Get-Content -LiteralPath (Join-Path $Output "$name.pe.txt")) {
            if ($line -match '^Import \{') { $iat = 0L }
            if ($line -match '^  Name: (.+)$') { $dll = $Matches[1] }
            if ($line -match 'ImportAddressTableRVA: 0x([0-9A-Fa-f]+)') { $iat = $imageBase + [Convert]::ToInt64($Matches[1],16) }
            if ($iat -ne 0 -and $line -match '^  Symbol: (.*?) \((\d+)\)') {
                $symbol = if ($Matches[1]) { $Matches[1] } else { '#' + $Matches[2] }
                $labels[('0x{0:x}' -f $iat)] = "$dll!$symbol"
                $iat += 8
            }
        }
        Get-Content -LiteralPath (Join-Path $Output 'WinMail.disasm.txt') | ForEach-Object {
            $line = $_
            if ($line -match '# (0x[0-9a-f]+)') {
                $key = $Matches[1]
                if ($labels.ContainsKey($key)) { $line += ' ; ' + $labels[$key] }
            }
            $line
        } | Set-Content -LiteralPath (Join-Path $Output 'WinMail.annotated.asm') -Encoding utf8
    }
    $manifest += [pscustomobject]@{ File = $path; SHA256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash; Size = $bytes.Length; Version = (Get-Item -LiteralPath $path).VersionInfo.FileVersion }
}
$manifest | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $Output 'inputs.json') -Encoding utf8
