use std::sync::atomic::{AtomicBool, AtomicU64};

#[derive(Default)]
pub(crate) struct AppRuntime {
    pub(super) is_quitting: AtomicBool,
    pub(crate) window_state_save_generation: AtomicU64,
    pub(crate) window_state_save_worker_running: AtomicBool,
}
