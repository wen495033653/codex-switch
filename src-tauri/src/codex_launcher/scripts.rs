pub(crate) const GET_CODEX_PACKAGE_INFO: &str = r#"
$ErrorActionPreference = "Stop"
$package = Get-AppxPackage OpenAI.Codex | Where-Object { $_.InstallLocation } | Sort-Object Version -Descending | Select-Object -First 1
if (-not $package) { throw "OpenAI.Codex package was not found." }
$manifest = Get-AppxPackageManifest -Package $package.PackageFullName
if (-not $manifest) { throw "OpenAI.Codex manifest could not be loaded." }
$application = @($manifest.Package.Applications.Application) | Select-Object -First 1
if (-not $application) { throw "OpenAI.Codex manifest does not contain an Application entry." }
$applicationId = [string]$application.Id
if ([string]::IsNullOrWhiteSpace($applicationId)) { throw "OpenAI.Codex manifest Application Id is empty." }
$executable = Join-Path $package.InstallLocation ([string]$application.Executable)
if (-not (Test-Path -LiteralPath $executable)) { throw "Codex executable was not found: $executable" }
$visual = $application.VisualElements
$logoCandidates = @(
  "assets/Square44x44Logo.targetsize-256_altform-unplated.png",
  "assets/Square44x44Logo.targetsize-256_altform-lightunplated.png",
  [string]$visual.Square150x150Logo,
  [string]$visual.Square44x44Logo,
  [string]$visual.Logo,
  [string]$visual.SmallLogo
) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) }
$iconPath = ""
foreach ($candidate in $logoCandidates) {
  $path = Join-Path $package.InstallLocation $candidate
  if (Test-Path -LiteralPath $path) {
    $iconPath = $path
    break
  }
}
[pscustomobject]@{
  PackageFamilyName = $package.PackageFamilyName
  InstallLocation = $package.InstallLocation
  ExecutablePath = $executable
  IconPath = $iconPath
  AppUserModelId = ("{0}!{1}" -f $package.PackageFamilyName, $applicationId)
} | ConvertTo-Json -Compress
"#;

pub(crate) const GET_CODEX_DESKTOP_PROCESSES: &str = r#"
$ErrorActionPreference = "Stop"
@(Get-CimInstance Win32_Process | Where-Object {
  $path = $_.ExecutablePath
  ($path -and $path -like "*\OpenAI.Codex_*\app\Codex.exe") -or
  ($path -and $path -like "*\OpenAI.Codex_*\app\resources\codex.exe")
} | Select-Object ProcessId, Name, ExecutablePath, CommandLine) | ConvertTo-Json -Compress
"#;

pub(crate) const CAPTURE_OPEN_IDE_SNAPSHOT: &str = r#"
$ErrorActionPreference = "Stop"
$names = @("Codex.exe", "Code.exe")
$list = Get-CimInstance Win32_Process | Where-Object {
  $_.ExecutablePath -and $_.Name -and $names -contains $_.Name
} | Select-Object ProcessId, Name, ExecutablePath
$list | ConvertTo-Json -Depth 2 -Compress
"#;

pub(crate) fn alive_pids(pids: &[u64]) -> String {
    let joined = pids
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        r#"
$ErrorActionPreference = "Stop"
$ids = @({joined})
$alive = Get-Process -Id $ids -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Id
$alive | ConvertTo-Json -Depth 2 -Compress
"#
    )
}

pub(crate) fn codex_proxy_connection(process_ids: &[u64], port: u16) -> String {
    let process_id_list = process_ids
        .iter()
        .map(u64::to_string)
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"
$processIds = @({process_id_list})
$port = {port}
$connections = @(Get-NetTCPConnection -ErrorAction SilentlyContinue | Where-Object {{
  ($processIds -contains $_.OwningProcess) -and (($_.RemotePort -eq $port) -or ($_.LocalPort -eq $port))
}})
[pscustomobject]@{{ Connected = ($connections.Count -gt 0) }} | ConvertTo-Json -Compress
"#
    )
}
