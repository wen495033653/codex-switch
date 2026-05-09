use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};

pub(crate) struct OAuthFlowControl {
    flow_id: u64,
    canceled: Arc<AtomicBool>,
}

#[derive(Default)]
pub(crate) struct OAuthRuntime {
    current: Mutex<Option<OAuthFlowControl>>,
    seq: AtomicU64,
}

pub(super) fn start_oauth_flow(runtime: &OAuthRuntime) -> Result<(u64, Arc<AtomicBool>), String> {
    let mut current = runtime
        .current
        .lock()
        .map_err(|_| "OAuth runtime lock poisoned".to_string())?;
    if current.is_some() {
        return Err("OAuth 登录流程进行中，请勿重复点击".to_string());
    }
    let flow_id = runtime.seq.fetch_add(1, Ordering::SeqCst) + 1;
    let canceled = Arc::new(AtomicBool::new(false));
    *current = Some(OAuthFlowControl {
        flow_id,
        canceled: Arc::clone(&canceled),
    });
    Ok((flow_id, canceled))
}

pub(super) fn finish_oauth_flow(runtime: &OAuthRuntime, flow_id: u64) {
    if let Ok(mut current) = runtime.current.lock() {
        if current.as_ref().is_some_and(|flow| flow.flow_id == flow_id) {
            *current = None;
        }
    }
}

pub(super) fn cancel_oauth_flow(runtime: &OAuthRuntime) -> bool {
    runtime
        .current
        .lock()
        .ok()
        .and_then(|current| {
            current.as_ref().map(|flow| {
                flow.canceled.store(true, Ordering::SeqCst);
                true
            })
        })
        .unwrap_or(false)
}
