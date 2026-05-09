use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    mpsc::{self, Receiver, Sender},
    Arc, Mutex,
};
use tauri::State;

pub(crate) struct OAuthFlowControl {
    flow_id: u64,
    canceled: Arc<AtomicBool>,
    callback_sender: Sender<String>,
}

#[derive(Default)]
pub(crate) struct OAuthRuntime {
    current: Mutex<Option<OAuthFlowControl>>,
    seq: AtomicU64,
}

pub(super) fn start_oauth_flow(
    runtime: &State<'_, OAuthRuntime>,
) -> Result<(u64, Arc<AtomicBool>, Receiver<String>), String> {
    let mut current = runtime
        .current
        .lock()
        .map_err(|_| "OAuth runtime lock poisoned".to_string())?;
    if current.is_some() {
        return Err("OAuth 登录流程进行中，请勿重复点击".to_string());
    }
    let flow_id = runtime.seq.fetch_add(1, Ordering::SeqCst) + 1;
    let canceled = Arc::new(AtomicBool::new(false));
    let (callback_sender, callback_receiver) = mpsc::channel();
    *current = Some(OAuthFlowControl {
        flow_id,
        canceled: Arc::clone(&canceled),
        callback_sender,
    });
    Ok((flow_id, canceled, callback_receiver))
}

pub(super) fn finish_oauth_flow(runtime: &State<'_, OAuthRuntime>, flow_id: u64) {
    if let Ok(mut current) = runtime.current.lock() {
        if current.as_ref().is_some_and(|flow| flow.flow_id == flow_id) {
            *current = None;
        }
    }
}

pub(super) fn cancel_oauth_flow(runtime: &State<'_, OAuthRuntime>) -> bool {
    runtime
        .current
        .lock()
        .ok()
        .and_then(|mut current| current.take())
        .map(|flow| {
            flow.canceled.store(true, Ordering::SeqCst);
            true
        })
        .unwrap_or(false)
}

pub(super) fn submit_oauth_callback(
    runtime: &State<'_, OAuthRuntime>,
    callback_url: String,
) -> Result<(), String> {
    let current = runtime
        .current
        .lock()
        .map_err(|_| "OAuth runtime lock poisoned".to_string())?;
    let flow = current
        .as_ref()
        .ok_or_else(|| "当前没有进行中的 OAuth 登录".to_string())?;
    flow.callback_sender
        .send(callback_url)
        .map_err(|_| "OAuth 登录流程已结束，请重新开始".to_string())
}
