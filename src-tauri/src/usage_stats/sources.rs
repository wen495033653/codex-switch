use super::*;

pub(super) fn main_usage_scan_source(codex_home: &Path) -> UsageScanSource {
    UsageScanSource {
        codex_home: codex_home.to_path_buf(),
        attribution_override: None,
    }
}

pub(super) fn default_usage_scan_sources(
    codex_home: &Path,
) -> Result<Vec<UsageScanSource>, String> {
    let mut sources = vec![main_usage_scan_source(codex_home)];
    let instances_dir = app_data_dir()?.join(CODEX_APP_INSTANCES_DIR);
    sources.extend(managed_instance_usage_scan_sources(&instances_dir)?);
    Ok(sources)
}

pub(super) fn managed_instance_usage_scan_sources(
    instances_dir: &Path,
) -> Result<Vec<UsageScanSource>, String> {
    if !instances_dir.exists() {
        return Ok(Vec::new());
    }

    let entries = fs::read_dir(instances_dir).map_err(|err| {
        format!(
            "读取 Codex 多开实例目录失败 {}: {err}",
            instances_dir.display()
        )
    })?;
    let mut sources = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| format!("读取 Codex 多开实例条目失败: {err}"))?;
        let root = entry.path();
        if !root.is_dir() {
            continue;
        }
        let Some(attribution) = read_instance_owner_attribution(&root)? else {
            continue;
        };
        sources.push(UsageScanSource {
            codex_home: root.join("codex-home"),
            attribution_override: Some(attribution),
        });
    }
    sources.sort_by(|left, right| left.codex_home.cmp(&right.codex_home));
    Ok(sources)
}

pub(super) fn read_instance_owner_attribution(
    root: &Path,
) -> Result<Option<OwnerAttribution>, String> {
    let marker_path = root.join(CODEX_APP_INSTANCE_MARKER_FILE);
    if !marker_path.exists() {
        return Ok(None);
    }
    let raw = fs::read_to_string(&marker_path).map_err(|err| {
        format!(
            "读取 Codex 多开实例标记失败 {}: {err}",
            marker_path.display()
        )
    })?;
    let marker: Value = serde_json::from_str(&raw).map_err(|err| {
        format!(
            "解析 Codex 多开实例标记失败 {}: {err}",
            marker_path.display()
        )
    })?;
    if string_field(&marker, "managedBy") != "codex-switch" {
        return Ok(None);
    }

    let target_id = string_field(&marker, "targetId");
    if target_id.is_empty() {
        return Err(format!(
            "Codex 多开实例标记缺少 targetId: {}",
            marker_path.display()
        ));
    }

    let owner_type = match string_field(&marker, "kind").as_str() {
        "account" => OWNER_TYPE_SUBSCRIPTION,
        "api" => OWNER_TYPE_API_PROFILE,
        _ => {
            return Err(format!(
                "Codex 多开实例标记 kind 无效: {}",
                marker_path.display()
            ))
        }
    };

    Ok(Some(OwnerAttribution {
        owner_type: owner_type.to_string(),
        owner_id: target_id,
    }))
}
