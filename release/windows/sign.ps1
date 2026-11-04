param(
    [string]$Target = 'x86_64-pc-windows-msvc',
    [string[]]$FilePath = @()
)
$ErrorActionPreference = 'Stop'
if (-not $env:WINDOWS_CERTIFICATE_THUMBPRINT) { return }
$root = (Resolve-Path "$PSScriptRoot/../..").Path
if (-not $FilePath.Count) {
    $FilePath = @("$root/target/$Target/release/winc.exe", "$root/target/$Target/release/winc-pty.exe")
}
$signtool = Get-ChildItem "${env:ProgramFiles(x86)}\Windows Kits\10\bin\*\x64\signtool.exe" |
    Where-Object { $_.Directory.Parent.Name -as [version] } |
    Sort-Object { [version]$_.Directory.Parent.Name } -Descending | Select-Object -First 1
if (-not $signtool) { throw 'signtool.exe not found' }
foreach ($file in $FilePath) {
    & $signtool.FullName sign /fd sha256 /sha1 $env:WINDOWS_CERTIFICATE_THUMBPRINT /t http://timestamp.digicert.com /d winc $file
    if ($LASTEXITCODE -ne 0) { throw "signtool failed with $LASTEXITCODE" }
}
