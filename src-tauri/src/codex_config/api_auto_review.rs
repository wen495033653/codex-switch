//! API approvals use the selected conversation model, without changing the source catalog.

use super::parse::{read_root_config, root_config_from_lines};
use super::write::{remove_config_values, set_config_values};
use crate::{app_log::log_event, atomic_file::write_file_atomically, paths::codex_dir};
use serde_json::{json, Map, Value};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

const CATALOG_KEY: &str = "model_catalog_json";
const MANAGED_FILE: &str = "codex-switch-api-models.json";
const ORIGINAL_KEY: &str = "original_model_catalog_json";

#[derive(Debug, PartialEq)]
pub(crate) enum CatalogRestore {
    Unchanged,
    Set(String),
    Remove,
}

fn configured_catalog(root: &Map<String, Value>) -> Option<&str> {
    root.get(CATALOG_KEY)
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
}

fn resolve_catalog(home: &Path, configured: &str) -> PathBuf {
    let path = Path::new(configured);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        home.join(path)
    }
}

fn path_for_config(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn is_managed_catalog(home: &Path, configured: &str) -> bool {
    let current = path_for_config(&resolve_catalog(home, configured));
    let managed = path_for_config(&home.join(MANAGED_FILE));
    #[cfg(windows)]
    {
        current.eq_ignore_ascii_case(&managed)
    }
    #[cfg(not(windows))]
    {
        current == managed
    }
}

fn read_catalog(path: &Path) -> Result<Value, String> {
    let raw = fs::read_to_string(path)
        .map_err(|err| format!("读取模型目录失败 {}: {err}", path.display()))?;
    serde_json::from_str(&raw).map_err(|err| format!("解析模型目录失败 {}: {err}", path.display()))
}

fn original_catalog(home: &Path, current: Option<&str>) -> Result<Option<String>, String> {
    let Some(current) = current else {
        return Ok(None);
    };
    if !is_managed_catalog(home, current) {
        return Ok(Some(current.to_string()));
    }
    let path = home.join(MANAGED_FILE);
    let catalog = read_catalog(&path)?;
    match catalog
        .get("_codex_switch")
        .and_then(|state| state.get(ORIGINAL_KEY))
    {
        Some(Value::Null) => Ok(None),
        Some(Value::String(original)) if !original.is_empty() => Ok(Some(original.clone())),
        _ => Err(format!(
            "模型目录缺少原始 {CATALOG_KEY}: {}",
            path.display()
        )),
    }
}

fn source_catalog(home: &Path) -> Result<PathBuf, String> {
    let path = home.join("config.toml");
    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == ErrorKind::NotFound => String::new(),
        Err(err) => return Err(format!("读取模型目录配置失败 {}: {err}", path.display())),
    };
    let lines: Vec<String> = raw.lines().map(str::to_string).collect();
    let root = root_config_from_lines(&lines);
    Ok(match configured_catalog(&root) {
        Some(configured) => resolve_catalog(home, configured),
        None => home.join("models_cache.json"),
    })
}

pub(crate) fn prepare_api_auto_review_catalog_for_home(
    home: &Path,
    root: &Map<String, Value>,
    source_home: &Path,
) -> Result<String, String> {
    let managed = home.join(MANAGED_FILE);
    let mut source = None;
    let result = (|| {
        let original = original_catalog(home, configured_catalog(root))?;
        let source_path = match original.as_deref() {
            Some(configured) => resolve_catalog(home, configured),
            None if home == source_home => home.join("models_cache.json"),
            None => source_catalog(source_home)?,
        };
        source = Some(path_for_config(&source_path));
        let catalog = read_catalog(&source_path)?;
        let mut models = catalog
            .get("models")
            .and_then(Value::as_array)
            .cloned()
            .filter(|models| !models.is_empty())
            .ok_or_else(|| format!("模型目录缺少非空 models 数组: {}", source_path.display()))?;
        for (index, model) in models.iter_mut().enumerate() {
            let slug = model
                .get("slug")
                .and_then(Value::as_str)
                .filter(|slug| !slug.is_empty())
                .ok_or_else(|| {
                    format!(
                        "模型目录 models[{index}] 缺少 slug: {}",
                        source_path.display()
                    )
                })?
                .to_string();
            // Each entry follows itself, so changing the conversation model also changes approvals.
            model
                .as_object_mut()
                .ok_or_else(|| {
                    format!(
                        "模型目录 models[{index}] 不是对象: {}",
                        source_path.display()
                    )
                })?
                .insert(
                    "auto_review_model_override".to_string(),
                    Value::String(slug),
                );
        }
        let next = json!({"models": models, "_codex_switch": {ORIGINAL_KEY: original}});
        let raw = serde_json::to_vec_pretty(&next)
            .map_err(|err| format!("序列化 API 模型目录失败 {}: {err}", managed.display()))?;
        let unchanged = match fs::read(&managed) {
            Ok(current) => current == raw,
            Err(err) if err.kind() == ErrorKind::NotFound => false,
            Err(err) => {
                return Err(format!("读取受管模型目录失败 {}: {err}", managed.display()));
            }
        };
        if !unchanged {
            write_file_atomically(&managed, &raw)?;
        }
        Ok(path_for_config(&managed))
    })();
    if let Err(error) = &result {
        log_event(
            "codex_api_auto_review_catalog_error",
            json!({
                "operation": "prepare", "catalog": managed, "source": source,
                "model": root.get("model"), "error": error
            }),
        );
    }
    result
}

pub(crate) fn prepare_current_api_auto_review_catalog() -> Result<String, String> {
    let home = codex_dir()?;
    prepare_api_auto_review_catalog_for_home(&home, &read_current_catalog_root(&home)?, &home)
}

fn read_current_catalog_root(home: &Path) -> Result<Map<String, Value>, String> {
    let result = read_root_config();
    if let Err(error) = &result {
        log_event(
            "codex_api_auto_review_catalog_error",
            json!({
                "operation": "read_config", "catalog": home.join(MANAGED_FILE),
                "source": home.join("config.toml"), "error": error
            }),
        );
    }
    result
}

pub(crate) fn api_auto_review_catalog_restore(
    home: &Path,
    root: &Map<String, Value>,
) -> Result<CatalogRestore, String> {
    let Some(current) =
        configured_catalog(root).filter(|current| is_managed_catalog(home, current))
    else {
        return Ok(CatalogRestore::Unchanged);
    };
    let result = original_catalog(home, Some(current)).map(|original| match original {
        Some(original) => CatalogRestore::Set(original),
        None => CatalogRestore::Remove,
    });
    if let Err(error) = &result {
        log_event(
            "codex_api_auto_review_catalog_error",
            json!({
                "operation": "restore", "catalog": home.join(MANAGED_FILE),
                "model": root.get("model"), "error": error
            }),
        );
    }
    result
}

pub(crate) fn current_auto_review_catalog_restore() -> Result<CatalogRestore, String> {
    let home = codex_dir()?;
    api_auto_review_catalog_restore(&home, &read_current_catalog_root(&home)?)
}

pub(crate) fn apply_auto_review_catalog_restore(restore: CatalogRestore) -> Result<(), String> {
    match restore {
        CatalogRestore::Unchanged => Ok(()),
        CatalogRestore::Set(original) => set_config_values(vec![(CATALOG_KEY, original)]),
        CatalogRestore::Remove => remove_config_values(&[CATALOG_KEY]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn fixture(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let home = std::env::temp_dir().join(format!("codex-switch-auto-review-{name}-{stamp}"));
        fs::create_dir_all(&home).unwrap();
        home
    }

    fn root(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn api_catalog_follows_each_parent_and_retains_source_and_original_pointer() {
        let home = fixture("custom");
        let source = home.join("custom.json");
        let raw = r#"{"models":[{"slug":"parent-a","context_window":123},{"slug":"parent-b","auto_review_model_override":"old-reviewer"}]}"#;
        fs::write(&source, raw).unwrap();
        let config = root(json!({"model_catalog_json":"custom.json"}));
        let managed = prepare_api_auto_review_catalog_for_home(&home, &config, &home).unwrap();
        assert_eq!(fs::read_to_string(&source).unwrap(), raw);
        let value = read_catalog(Path::new(&managed)).unwrap();
        assert_eq!(value["models"][0]["context_window"], 123);
        assert_eq!(value["models"][0]["auto_review_model_override"], "parent-a");
        assert_eq!(value["models"][1]["auto_review_model_override"], "parent-b");
        let active = root(json!({"model_catalog_json": managed}));
        let modified = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        fs::OpenOptions::new()
            .write(true)
            .open(&managed)
            .unwrap()
            .set_modified(modified)
            .unwrap();
        prepare_api_auto_review_catalog_for_home(&home, &active, &home).unwrap();
        assert_eq!(
            fs::metadata(&managed).unwrap().modified().unwrap(),
            modified
        );
        assert_eq!(
            api_auto_review_catalog_restore(&home, &active).unwrap(),
            CatalogRestore::Set("custom.json".into())
        );
        fs::write(&source, r#"{"models":[{"slug":"new-parent"}]}"#).unwrap();
        prepare_api_auto_review_catalog_for_home(&home, &active, &home).unwrap();
        assert_eq!(
            read_catalog(Path::new(&managed)).unwrap()["models"][0]["slug"],
            "new-parent"
        );
    }

    #[test]
    fn api_catalog_uses_explicit_cache_and_restores_absent_pointer() {
        let home = fixture("cache");
        fs::write(
            home.join("models_cache.json"),
            r#"{"models":[{"slug":"parent"}]}"#,
        )
        .unwrap();
        let managed = prepare_api_auto_review_catalog_for_home(&home, &Map::new(), &home).unwrap();
        let active = root(json!({"model_catalog_json": managed}));
        assert_eq!(
            api_auto_review_catalog_restore(&home, &active).unwrap(),
            CatalogRestore::Remove
        );
    }

    #[test]
    fn subscription_does_not_replace_a_user_changed_catalog() {
        let home = fixture("manual");
        let config = root(json!({"model_catalog_json":"user-changed.json"}));
        assert_eq!(
            api_auto_review_catalog_restore(&home, &config).unwrap(),
            CatalogRestore::Unchanged
        );
    }

    #[test]
    fn subscription_rejects_missing_or_corrupt_restore_metadata_without_writing() {
        let home = fixture("restore-error");
        let managed = home.join(MANAGED_FILE);
        let config = root(json!({"model_catalog_json": path_for_config(&managed)}));
        assert!(api_auto_review_catalog_restore(&home, &config)
            .unwrap_err()
            .contains("读取模型目录失败"));
        for raw in ["{", r#"{"models":[]}"#] {
            fs::write(&managed, raw).unwrap();
            let error = api_auto_review_catalog_restore(&home, &config).unwrap_err();
            assert!(
                error.contains("解析模型目录失败") || error.contains("缺少原始 model_catalog_json")
            );
            assert_eq!(fs::read_to_string(&managed).unwrap(), raw);
        }
    }

    #[test]
    fn independent_instance_gets_its_own_catalog_and_restore_state() {
        let source_home = fixture("global");
        fs::write(
            source_home.join("config.toml"),
            "model_catalog_json = \"custom.json\"\n",
        )
        .unwrap();
        fs::write(
            source_home.join("custom.json"),
            r#"{"models":[{"slug":"parent"}]}"#,
        )
        .unwrap();
        let global = root(json!({"model_catalog_json":"custom.json"}));
        let global_managed =
            prepare_api_auto_review_catalog_for_home(&source_home, &global, &source_home).unwrap();
        fs::write(
            source_home.join("config.toml"),
            format!(
                "model_catalog_json = {}\n",
                serde_json::to_string(&global_managed).unwrap()
            ),
        )
        .unwrap();
        let home = fixture("instance");
        let managed =
            prepare_api_auto_review_catalog_for_home(&home, &Map::new(), &source_home).unwrap();
        assert_eq!(Path::new(&managed).parent(), Some(home.as_path()));
        assert_eq!(
            read_catalog(Path::new(&managed)).unwrap()["_codex_switch"][ORIGINAL_KEY],
            Value::Null
        );
        let active = root(json!({"model_catalog_json": managed}));
        assert_eq!(
            api_auto_review_catalog_restore(&home, &active).unwrap(),
            CatalogRestore::Remove
        );
    }

    #[test]
    fn missing_or_invalid_catalog_is_returned_and_logged_with_the_source() {
        let home = fixture("invalid");
        let config = root(json!({"model_catalog_json":"missing.json", "model":"parent"}));
        let error = prepare_api_auto_review_catalog_for_home(&home, &config, &home).unwrap_err();
        assert!(error.contains("missing.json"));
        assert!(!home.join(MANAGED_FILE).exists());
        let entries = crate::app_log::get_dev_log_entries();
        assert!(entries.as_array().unwrap().iter().any(|entry| {
            entry["details"]["event"] == "codex_api_auto_review_catalog_error"
                && entry["details"]["details"]["source"]
                    .as_str()
                    .is_some_and(|path| path.ends_with("missing.json"))
                && entry["details"]["details"]["error"].as_str() == Some(error.as_str())
        }));
        fs::write(home.join("missing.json"), "{").unwrap();
        assert!(
            prepare_api_auto_review_catalog_for_home(&home, &config, &home)
                .unwrap_err()
                .contains("解析模型目录失败")
        );
    }

    #[test]
    #[ignore = "requires an explicit isolated fixture catalog for real CLI routing verification"]
    fn build_isolated_api_auto_review_catalog() {
        let home = PathBuf::from(
            std::env::var_os("CODEX_SWITCH_AUTO_REVIEW_TEST_HOME")
                .expect("explicit fixture home required"),
        );
        let config = root(json!({"model_catalog_json":"input-models.json"}));
        let managed = prepare_api_auto_review_catalog_for_home(&home, &config, &home).unwrap();
        println!("managed_catalog={managed}");
    }
}
