use crate::session_sync_diagnostics::log_session_sync_event;
use serde_json::json;

/// Tauri runs a non-async command on the main thread, so file, process or network work there
/// freezes the window. Commands hand such work to the blocking pool through this helper; only
/// the join result is awaited.
pub(crate) async fn run_blocking<T: Send + 'static>(
    action: &'static str,
    task: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|err| {
            let raw = err.to_string();
            log_session_sync_event(
                "command_blocking_task_error",
                json!({ "action": action, "error": raw }),
            );
            blocking_task_error(action, &raw)
        })?
}

fn blocking_task_error(action: &str, raw: &str) -> String {
    if raw.contains("panicked") {
        format!("{action}任务异常，请重试")
    } else {
        format!("{action}任务异常: {raw}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn task_runs_off_calling_thread_and_returns_its_result() {
        let caller = thread::current().id();
        let value = tauri::async_runtime::block_on(run_blocking("fixture", move || {
            assert_ne!(caller, thread::current().id());
            Ok(7)
        }))
        .unwrap();
        assert_eq!(value, 7);

        let error = tauri::async_runtime::block_on(run_blocking("fixture", || -> Result<(), _> {
            Err("fixture: worker failed".to_string())
        }))
        .unwrap_err();
        assert_eq!(error, "fixture: worker failed");
    }

    #[test]
    fn panicking_task_is_returned_as_error() {
        let error = tauri::async_runtime::block_on(run_blocking("fixture", || -> Result<(), _> {
            panic!("fixture panic")
        }))
        .unwrap_err();
        assert_eq!(error, "fixture任务异常，请重试");
    }
}
