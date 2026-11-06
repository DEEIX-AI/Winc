# Adds a directory to the current user's Path or removes it. Reads and writes the registry value
# directly: the Environment class expands %VAR% entries and would rewrite them.
param(
    [Parameter(Mandatory)][ValidateSet('Add', 'Remove')][string]$Action,
    [Parameter(Mandatory)][string]$Directory
)
$ErrorActionPreference = 'Stop'
$key = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment')
try {
    $options = [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames
    $current = [string]$key.GetValue('Path', '', $options)
    $wanted = $Directory.TrimEnd('\')
    $entries = @($current -split ';' | Where-Object { $_ -and $_.TrimEnd('\') -ne $wanted })
    if ($Action -eq 'Add') { $entries += $wanted }
    $kind = [Microsoft.Win32.RegistryValueKind]::ExpandString
    $key.SetValue('Path', ($entries -join ';'), $kind)
} finally {
    $key.Close()
}
# Deleting an absent variable changes nothing and tells running programs to reread the environment.
[Environment]::SetEnvironmentVariable('WINC_PATH_CHANGED', $null, 'User')
