use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize)]
pub(super) struct ConversationItem {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) updated_at: Option<String>,
    pub(super) status: String,
    pub(super) source_path: String,
    pub(super) relative_path: String,
    pub(super) size_bytes: u64,
    pub(super) cwd: Option<String>,
    pub(super) preview: Option<String>,
    pub(super) sha256: Option<String>,
    pub(super) parse_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct ConversationMessage {
    pub(super) role: String,
    pub(super) text: String,
    pub(super) timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) offset: Option<u64>,
}

#[derive(Debug, Default, Clone)]
pub(super) struct SessionSummary {
    pub(super) id: Option<String>,
    pub(super) title: Option<String>,
    pub(super) created_at: Option<String>,
    pub(super) updated_at: Option<String>,
    pub(super) cwd: Option<String>,
    pub(super) source: Option<String>,
    pub(super) thread_source: Option<String>,
    pub(super) model_provider: Option<String>,
    pub(super) sandbox_policy: Option<String>,
    pub(super) approval_mode: Option<String>,
    pub(super) cli_version: Option<String>,
    pub(super) agent_nickname: Option<String>,
    pub(super) agent_role: Option<String>,
    pub(super) agent_path: Option<String>,
    pub(super) history_mode: Option<String>,
    pub(super) parent_thread_id: Option<String>,
    pub(super) model: Option<String>,
    pub(super) reasoning_effort: Option<String>,
    pub(super) first_user_message: Option<String>,
    pub(super) preview: Option<String>,
    pub(super) dynamic_tools: Vec<ThreadDynamicToolMetadata>,
    pub(super) messages: Vec<ConversationMessage>,
    pub(super) parse_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct ManifestSession {
    pub(super) id: String,
    pub(super) title: String,
    pub(super) updated_at: Option<String>,
    pub(super) status: String,
    pub(super) relative_path: String,
    pub(super) size_bytes: u64,
    pub(super) sha256: String,
}

#[derive(Debug, Clone)]
pub(super) struct ThreadMetadata {
    pub(super) id: String,
    pub(super) rollout_path: PathBuf,
    pub(super) created_at: i64,
    pub(super) updated_at: i64,
    pub(super) source: String,
    pub(super) model_provider: String,
    pub(super) cwd: String,
    pub(super) title: String,
    pub(super) sandbox_policy: String,
    pub(super) approval_mode: String,
    pub(super) has_user_event: i64,
    pub(super) archived: i64,
    pub(super) archived_at: Option<i64>,
    pub(super) cli_version: String,
    pub(super) first_user_message: String,
    pub(super) agent_nickname: Option<String>,
    pub(super) agent_role: Option<String>,
    pub(super) model: Option<String>,
    pub(super) reasoning_effort: Option<String>,
    pub(super) agent_path: Option<String>,
    pub(super) thread_source: Option<String>,
    pub(super) preview: String,
    pub(super) history_mode: String,
    pub(super) parent_thread_id: Option<String>,
    pub(super) dynamic_tools: Vec<ThreadDynamicToolMetadata>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct ThreadDynamicToolMetadata {
    pub(super) name: String,
    pub(super) description: String,
    pub(super) input_schema: String,
    pub(super) defer_loading: bool,
    pub(super) namespace: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct StatusMove {
    pub(super) id: String,
    pub(super) target_id: String,
    pub(super) source_path: PathBuf,
    pub(super) target_path: PathBuf,
    pub(super) rewrite_id: Option<(String, String)>,
    pub(super) overwritten_id: Option<String>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(super) enum ConflictStrategy {
    Ask,
    Skip,
    Overwrite,
    ModifyId,
}

pub(super) fn parse_conflict_strategy(value: Option<String>) -> Result<ConflictStrategy, String> {
    match value
        .as_deref()
        .unwrap_or("ask")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "" | "ask" => Ok(ConflictStrategy::Ask),
        "skip" => Ok(ConflictStrategy::Skip),
        "overwrite" => Ok(ConflictStrategy::Overwrite),
        "modify_id" | "modify-id" | "modifyid" | "reassign_id" | "reassign-id" | "reassignid" => {
            Ok(ConflictStrategy::ModifyId)
        }
        other => Err(format!("不支持的冲突处理方式: {other}")),
    }
}
