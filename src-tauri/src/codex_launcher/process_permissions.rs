use crate::session_sync_diagnostics::PROCESS_ELEVATION_WARNING;
use std::{
    io,
    mem::size_of,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr,
};
use windows_sys::Win32::{
    Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY},
    System::Threading::{OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION},
};

// Query tokens only: this module never requests elevation or terminates a process.
pub(super) fn ensure_termination_elevation(caller_pid: u64, target_pid: u64) -> Result<(), String> {
    let caller_elevated = process_is_elevated(caller_pid)?;
    let target_elevated = process_is_elevated(target_pid)?;
    check_elevation(caller_pid, caller_elevated, target_pid, target_elevated)
}

fn check_elevation(
    caller_pid: u64,
    caller_elevated: bool,
    target_pid: u64,
    target_elevated: bool,
) -> Result<(), String> {
    if !caller_elevated && target_elevated {
        return Err(format!(
            "{PROCESS_ELEVATION_WARNING} 权限预检查拒绝结束管理员进程：callerPid={caller_pid}, callerElevated={caller_elevated}, targetPid={target_pid}, targetElevated={target_elevated}; 未执行 taskkill，未结束子进程"
        ));
    }
    Ok(())
}

fn process_is_elevated(pid: u64) -> Result<bool, String> {
    let pid = u32::try_from(pid).map_err(|_| format!("无效的进程 PID: {pid}"))?;
    // Only query access is requested; the returned process and token handles are owned below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(format!(
            "权限预检查 OpenProcess PID={pid} 失败: {}",
            io::Error::last_os_error()
        ));
    }
    let process = unsafe { OwnedHandle::from_raw_handle(process) };
    let mut token = ptr::null_mut();
    if unsafe { OpenProcessToken(process.as_raw_handle(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(format!(
            "权限预检查 OpenProcessToken PID={pid} 失败: {}",
            io::Error::last_os_error()
        ));
    }
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
    let mut returned_size = 0;
    if unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenElevation,
            (&mut elevation as *mut TOKEN_ELEVATION).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned_size,
        )
    } == 0
    {
        return Err(format!(
            "权限预检查 GetTokenInformation(TokenElevation) PID={pid} 失败: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(elevation.TokenIsElevated != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn medium_caller_cannot_terminate_elevated_target() {
        let error = check_elevation(10, false, 20, true).unwrap_err();
        for evidence in [
            "callerPid=10",
            "callerElevated=false",
            "targetPid=20",
            "targetElevated=true",
            "未执行 taskkill",
        ] {
            assert!(error.contains(evidence), "{error}");
        }
        assert!(check_elevation(10, false, 20, false).is_ok());
        assert!(check_elevation(10, true, 20, true).is_ok());
        assert!(check_elevation(10, true, 20, false).is_ok());
    }

    #[test]
    fn current_process_token_is_readable() {
        let pid = u64::from(std::process::id());
        ensure_termination_elevation(pid, pid).unwrap();
    }

    #[test]
    fn token_query_failure_preserves_api_and_os_error() {
        let error = process_is_elevated(0).unwrap_err();
        assert!(error.contains("OpenProcess PID=0"), "{error}");
        assert!(error.contains("os error"), "{error}");
    }

    #[test]
    #[ignore = "read-only live token check; requires explicit caller and target PIDs"]
    fn live_medium_caller_elevated_target_is_rejected_without_termination() {
        let caller = std::env::var("CODEX_SWITCH_PREFLIGHT_CALLER_PID")
            .expect("explicit caller PID")
            .parse::<u64>()
            .unwrap();
        let target = std::env::var("CODEX_SWITCH_PREFLIGHT_TARGET_PID")
            .expect("explicit target PID")
            .parse::<u64>()
            .unwrap();
        let error = ensure_termination_elevation(caller, target).unwrap_err();
        assert!(error.contains("callerElevated=false"), "{error}");
        assert!(error.contains("targetElevated=true"), "{error}");
        println!("Read-only live permission preflight: {error}");
    }
}
