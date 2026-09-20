use serde_json::{json, Value};

// User-facing results only. Debug steps, scans and normal "disabled" skips stay out of this view.
// This projection supplies the brief result; the reader attaches the original log line separately.
pub(super) fn project(event: &str, details: &Value, level: &str, message: &str) -> Option<Value> {
    let permission_warning = level == "warn" && event.ends_with("_error");
    let updated = details.get("updated").and_then(Value::as_u64).unwrap_or(0);
    let (level, title, summary, action) = if permission_warning {
        (
            "warn",
            if details.get("retry") == Some(&Value::Bool(false)) {
                "自动处理已暂停"
            } else {
                "已保留 Codex 进程"
            },
            "Codex Switch 为普通权限，目标进程为管理员权限。本次关闭已跳过，未结束主进程或子进程。"
                .to_string(),
            "无需现在退出 Codex；请在当前任务结束后再处理权限差异。",
        )
    } else if level == "error" {
        (
            "error",
            match event {
                "codex_app_watcher_on_open_error" | "codex_app_watcher_on_open_panic_error" => {
                    "自动处理失败并已停止"
                }
                "session_sync_preflight_error"
                | "codex_app_open_handler_session_sync_preflight_error" => "会话同步检查失败",
                _ if message == "未知调试事件" => "Codex 操作失败",
                _ => message,
            },
            if details.get("retry") == Some(&Value::Bool(false)) {
                "自动处理已停止，本次 Codex Switch 运行期间不会再次自动重启 Codex。".to_string()
            } else {
                "该步骤未完成；展开详情查看失败依据。".to_string()
            },
            "",
        )
    } else {
        match event {
            "app_start" => (
                "success",
                "Codex Switch 已启动",
                "运行日志已启用。".to_string(),
                "",
            ),
            "session_sync_preflight_finish" if updated > 0 => (
                "warn",
                "发现待同步数据",
                format!("预检查发现 {updated} 项差异；这不是同步完成，数据尚未修改。"),
                "",
            ),
            "session_sync_preflight_finish" => (
                "success",
                "同步检查通过",
                "未发现需要同步的数据。".to_string(),
                "",
            ),
            "session_sync_finish" => (
                "success",
                "会话同步完成",
                format!("本次更新 {updated} 项。"),
                "",
            ),
            "codex_app_launch_confirmation_finish" => (
                "success",
                "Codex 启动检查通过",
                "新进程在检查期间持续运行；不代表界面或任务已就绪。".to_string(),
                "",
            ),
            "codex_app_process_kill_finish"
                if details.get("terminated") == Some(&Value::Bool(true)) =>
            {
                (
                    "success",
                    "进程关闭完成",
                    "已确认目标进程树退出。".to_string(),
                    "",
                )
            }
            "codex_app_relaunch_executable_finish"
                if details.get("restarted") != Some(&Value::Bool(true)) =>
            {
                (
                    "warn",
                    "Codex 未重新打开",
                    "本次操作未启动 Codex。".to_string(),
                    "",
                )
            }
            "codex_app_relaunch_processes_finish" | "codex_app_relaunch_executable_finish" => (
                "success",
                "Codex 重启流程完成",
                "本次重新打开流程已完成。".to_string(),
                "",
            ),
            "codex_app_relaunch_processes_post_exit_config_apply_finish" => (
                "success",
                "运行配置同步完成",
                "已在 Codex 退出后应用运行配置。".to_string(),
                "",
            ),
            "codex_remote_control_helper_spawn" => (
                "success",
                "远程控制进程已启动",
                "已创建 helper 进程。".to_string(),
                "",
            ),
            "codex_app_multi_open_finish" => (
                "success",
                "独立 Codex 已打开",
                "独立实例的启动流程已完成。".to_string(),
                "",
            ),
            "codex_app_multi_open_show_window" => (
                "success",
                "已显示 Codex 窗口",
                "已完成显示已有窗口的操作。".to_string(),
                "",
            ),
            "codex_app_instance_launch_skip" | "codex_app_relaunch_executable_skip"
                if details.get("reason").and_then(Value::as_str) == Some("missing_executable") =>
            {
                (
                    "warn",
                    "Codex 未启动",
                    "未找到 Codex 可执行文件。".to_string(),
                    "请检查 Codex 安装路径。",
                )
            }
            _ => return None,
        }
    };

    Some(json!({"level": level, "title": title, "summary": summary,
        "action": action}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preflight_pending_is_warning_not_sync_success() {
        let entry = project(
            "session_sync_preflight_finish",
            &json!({"updated": 7, "stateDbUpdated": 2,
            "rolloutFilesUpdated": 4, "globalStateUpdated": 1}),
            "debug",
            "",
        )
        .unwrap();
        assert_eq!(entry["level"], "warn");
        assert!(entry["summary"].as_str().unwrap().contains("数据尚未修改"));
        assert!(entry.get("fields").is_none());
        let sync = project("session_sync_finish", &json!({"updated": 7}), "debug", "").unwrap();
        assert_eq!(sync["level"], "success");
        assert_eq!(sync["title"], "会话同步完成");
    }

    #[test]
    fn summary_does_not_duplicate_raw_diagnostics() {
        assert!(project("session_sync_start", &json!({}), "debug", "").is_none());
        let entry = project(
            "codex_app_process_kill_error",
            &json!({"error": "exitCode=128; rawBase64=AAAA",
            "token": "fixture-private", "processes": [1, 2, 3]}),
            "error",
            "关闭失败",
        )
        .unwrap();
        let text = entry.to_string();
        assert!(!text.contains("exitCode=128"));
        assert!(!text.contains("AAAA"));
        assert!(!text.contains("fixture-private"));
        assert!(entry.get("fields").is_none());
    }

    #[test]
    fn unstarted_process_is_not_reported_as_success() {
        let entry = project(
            "codex_app_relaunch_executable_finish",
            &json!({"restarted": false}),
            "debug",
            "",
        )
        .unwrap();
        assert_eq!(entry["level"], "warn");
        assert!(project(
            "codex_app_process_kill_finish",
            &json!({"terminated": false}),
            "debug",
            ""
        )
        .is_none());
    }
}
