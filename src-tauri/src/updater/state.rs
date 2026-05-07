use std::sync::Mutex;
use tauri_plugin_updater::Update;

struct PendingUpdate {
    update: Update,
    bytes: Option<Vec<u8>>,
}

pub(super) type PendingUpdateData = Option<(Update, Option<Vec<u8>>)>;

#[derive(Default)]
pub(crate) struct UpdateRuntime {
    pending: Mutex<Option<PendingUpdate>>,
}

pub(super) fn clear_pending_update(runtime: &UpdateRuntime) {
    if let Ok(mut pending) = runtime.pending.lock() {
        *pending = None;
    }
}

pub(super) fn store_pending_update(
    runtime: &UpdateRuntime,
    update: Update,
    bytes: Option<Vec<u8>>,
) {
    if let Ok(mut pending) = runtime.pending.lock() {
        *pending = Some(PendingUpdate { update, bytes });
    }
}

pub(super) fn read_pending_update(runtime: &UpdateRuntime) -> Result<PendingUpdateData, String> {
    runtime
        .pending
        .lock()
        .map_err(|_| "更新状态锁异常".to_string())
        .map(|pending| {
            pending
                .as_ref()
                .map(|item| (item.update.clone(), item.bytes.clone()))
        })
}
