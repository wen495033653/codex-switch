pub(crate) const CAPTURE_OPEN_IDE_SNAPSHOT: &str = r#"
$ErrorActionPreference = "Stop"
$names = @("Codex.exe", "Code.exe")
$list = Get-CimInstance Win32_Process | Where-Object {
  $_.ExecutablePath -and $_.Name -and $names -contains $_.Name
} | Select-Object ProcessId, ParentProcessId, Name, ExecutablePath, CommandLine
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
