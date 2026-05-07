use std::sync::atomic::AtomicBool;

#[derive(Default)]
pub(crate) struct AppRuntime {
    pub(super) is_quitting: AtomicBool,
}
