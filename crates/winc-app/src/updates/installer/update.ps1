# Runs outside the application so Windows can replace both native executables.
param(
    [Parameter(Mandatory)][int]$ParentId,
    [Parameter(Mandatory)][string]$Setup,
    [Parameter(Mandatory)][string]$Executable,
    [Parameter(Mandatory)][ValidateSet('nsis', 'msi')][string]$Installer,
    [ValidateSet('0', '1')][string]$Relaunch = '0'
)
$ErrorActionPreference = 'Stop'
try {
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
    $_ | Out-String | Set-Content -Path (Join-Path (Split-Path -Parent $Setup) 'install-error.log')
    exit 1
}
