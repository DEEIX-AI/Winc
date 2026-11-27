# Runs outside the application so Windows can replace both native executables.
# Passed with -Command; the parameters come from WINC_UPDATE_* environment variables.
$ErrorActionPreference = 'Stop'
$ParentId = [int]$env:WINC_UPDATE_PARENT
$Setup = $env:WINC_UPDATE_SETUP
$Executable = $env:WINC_UPDATE_EXE
$Installer = $env:WINC_UPDATE_KIND
$Relaunch = $env:WINC_UPDATE_RELAUNCH
$Expected = $env:WINC_UPDATE_SHA256
$held = $null
function Get-Sha256([IO.Stream]$stream) {
    $sha = [Security.Cryptography.SHA256]::Create()
    try { -join ($sha.ComputeHash($stream) | ForEach-Object { $_.ToString('x2') }) }
    finally { $sha.Dispose() }
}
# Kept for the catch block: the environment variables are removed before the installer runs.
$Log = if ($Setup) { Join-Path (Split-Path -Parent $Setup) 'install-error.log' }
try {
    # The installer and the relaunched application inherit this environment.
    Remove-Item -Path Env:WINC_UPDATE_*
    $parent = Get-Process -Id $ParentId -ErrorAction SilentlyContinue
    if ($parent) { $parent.WaitForExit() }
    $directory = Split-Path -Parent $Executable
    # The package was verified when it was downloaded and has sat in the temporary directory
    # since. Open it without write or delete sharing, check it is still those bytes, and keep
    # it open until the installer has finished: nothing can replace it after this check.
    $held = [IO.File]::Open($Setup, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    if (-not $Expected -or (Get-Sha256 $held) -ne $Expected) {
        throw 'the update package changed after it was verified'
    }
    $before = (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash
    if ($Installer -eq 'msi') {
        $parameters = @{
            FilePath = "$env:SystemRoot/System32/msiexec.exe"
            ArgumentList = "/i `"$Setup`" /qn /norestart INSTALLFOLDER=`"$directory`""
            Wait = $true
            PassThru = $true
        }
        $principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
        if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { $parameters.Verb = 'RunAs' }
        $process = Start-Process @parameters
    } else {
        # NSIS requires /D to be last and unquoted; it consumes the remaining command line.
        $process = Start-Process -FilePath $Setup -ArgumentList "/S /D=$directory" -Wait -PassThru
    }
    if ($process.ExitCode -notin @(0, 1641, 3010)) { throw "installer failed with $($process.ExitCode)" }
    # The silent NSIS installer skips a file that is still in use and reports success all the
    # same. Windows Installer replaces such a file at the next restart and reports 1641 or 3010.
    $after = (Get-FileHash -LiteralPath $Executable -Algorithm SHA256).Hash
    if ($Installer -ne 'msi' -and $process.ExitCode -eq 0 -and $after -eq $before) {
        throw 'the installer left the application unchanged'
    }
    $held.Dispose()
    if ($Relaunch -eq '1') { Start-Process -FilePath $Executable -WorkingDirectory $directory }
    exit 0
} catch {
    if ($held) { $held.Dispose() }
    # Writing the log must not replace the error being reported: `$Setup` is empty when the
    # caller passed no environment variables, and `Split-Path -Parent $null` would throw here.
    if ($Log) {
        try { $_ | Out-String | Set-Content -Path $Log -ErrorAction Stop }
        catch { Write-Error "could not write $Log" -ErrorAction Continue }
    }
    # The user asked for the application to come back: start the copy that is still installed.
    if ($Relaunch -eq '1' -and $Executable -and (Test-Path -LiteralPath $Executable)) {
        try { Start-Process -FilePath $Executable -WorkingDirectory (Split-Path -Parent $Executable) }
        catch { Write-Error 'could not restart the application' -ErrorAction Continue }
    }
    exit 1
}
