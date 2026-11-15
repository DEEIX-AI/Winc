# Runs outside the application so Windows can replace both native executables.
# Passed with -Command; the parameters come from WINC_UPDATE_* environment variables.
$ErrorActionPreference = 'Stop'
$ParentId = [int]$env:WINC_UPDATE_PARENT
$Setup = $env:WINC_UPDATE_SETUP
$Executable = $env:WINC_UPDATE_EXE
$Installer = $env:WINC_UPDATE_KIND
$Relaunch = $env:WINC_UPDATE_RELAUNCH
# Kept for the catch block: the environment variables are removed before the installer runs.
$Log = if ($Setup) { Join-Path (Split-Path -Parent $Setup) 'install-error.log' }
try {
    # The installer and the relaunched application inherit this environment.
    Remove-Item -Path Env:WINC_UPDATE_*
    $parent = Get-Process -Id $ParentId -ErrorAction SilentlyContinue
    if ($parent) { $parent.WaitForExit() }
    $directory = Split-Path -Parent $Executable
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
    if ($Relaunch -eq '1') { Start-Process -FilePath $Executable -WorkingDirectory $directory }
    exit 0
} catch {
    # Writing the log must not replace the error being reported: `$Setup` is empty when the
    # caller passed no environment variables, and `Split-Path -Parent $null` would throw here.
    if ($Log) {
        try { $_ | Out-String | Set-Content -Path $Log -ErrorAction Stop }
        catch { Write-Error "could not write $Log" -ErrorAction Continue }
    }
    exit 1
}
