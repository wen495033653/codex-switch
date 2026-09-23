use super::super::api_mode::get_codex_state_value;
use super::model::{empty_store, normalize_store_data, profile_id_from_account};
use crate::{
    json_file::{read_json_file, write_json_file},
    json_util::raw_string_field,
    paths::accounts_path,
};
use serde_json::Value;
use std::{
    path::Path,
    sync::{Mutex, MutexGuard},
};

/// Serializes every access to accounts.json inside this process. Background refreshers,
/// commands and the OAuth flow all rewrite the whole file; without the lock a writer that
/// read the store earlier silently discards another writer's change, including a rotated
/// refresh_token that exists nowhere else.
static STORE_LOCK: Mutex<()> = Mutex::new(());

fn lock_store() -> MutexGuard<'static, ()> {
    // The lock guards the file, not in-memory data. Writes replace the file atomically, so a
    // panic while the lock was held left the file either untouched or fully written, and the
    // poison flag carries no state worth refusing later writes over.
    STORE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write_store_locked(path: &Path, store: &Value) -> Result<Value, String> {
    let normalized = normalize_store_data(store)?;
    write_json_file(path, "accounts.json", &normalized)?;
    Ok(normalized)
}

fn read_store_locked(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return write_store_locked(path, &empty_store());
    }

    let parsed = read_json_file(path, "accounts.json")?;
    normalize_store_data(&parsed)
}

pub(crate) fn read_store_value() -> Result<Value, String> {
    let path = accounts_path()?;
    let _guard = lock_store();
    read_store_locked(&path)
}

fn mutate_store_at<T>(
    path: &Path,
    update: impl FnOnce(&mut Value) -> Result<T, String>,
) -> Result<(Value, T), String> {
    let _guard = lock_store();
    let current = read_store_locked(path)?;
    let mut next = current.clone();
    let output = update(&mut next)?;
    if next == current {
        return Ok((current, output));
    }
    Ok((write_store_locked(path, &next)?, output))
}

/// Applies `update` to the store as it is on disk right now and writes the result back only
/// when something changed. Returns the stored value afterwards with `update`'s own output.
/// `update` must not call any other store function: the lock is not reentrant.
pub(crate) fn mutate_store<T>(
    update: impl FnOnce(&mut Value) -> Result<T, String>,
) -> Result<(Value, T), String> {
    mutate_store_at(&accounts_path()?, update)
}

pub(crate) fn read_store_with_active_sync() -> Result<Value, String> {
    let state = get_codex_state_value();
    let profile_id = raw_string_field(&state, "profile_id");
    if raw_string_field(&state, "mode") != "chatgpt" || profile_id.is_empty() {
        return read_store_value();
    }

    mutate_store(|store| {
        let known = store
            .get("accounts")
            .and_then(Value::as_array)
            .is_some_and(|accounts| {
                accounts.iter().any(|account| {
                    profile_id_from_account(account).unwrap_or_default() == profile_id
                })
            });
        if known {
            store["active_id"] = Value::String(profile_id);
        }
        Ok(())
    })
    .map(|(store, ())| store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::accounts::STORE_VERSION;
    use serde_json::json;
    use std::{
        env, fs,
        path::PathBuf,
        thread,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn temp_store_path(name: &str) -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = env::temp_dir().join(format!("codex-switch-store-{name}-{stamp}"));
        fs::create_dir_all(&dir).unwrap();
        dir.join("accounts.json")
    }

    fn account(account_id: &str) -> Value {
        json!({
            "tokens": {
                "id_token": format!("id-{account_id}"),
                "access_token": format!("access-{account_id}"),
                "refresh_token": format!("refresh-{account_id}"),
                "account_id": account_id
            },
            "custom": {
                "created_at": "2026-01-01T00:00:00Z",
                "last_used_at": "2026-01-01T00:00:00Z"
            }
        })
    }

    #[test]
    fn concurrent_mutations_keep_every_change() {
        let path = temp_store_path("concurrent");
        let handles = (0..8)
            .map(|index| {
                let path = path.clone();
                thread::spawn(move || {
                    mutate_store_at(&path, |store| {
                        let accounts = store["accounts"].as_array_mut().unwrap();
                        accounts.push(account(&format!("acct-{index}")));
                        Ok(())
                    })
                    .unwrap();
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            handle.join().unwrap();
        }

        let stored = read_store_locked(&path).unwrap();
        assert_eq!(stored["accounts"].as_array().unwrap().len(), 8);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn unchanged_store_is_not_rewritten() {
        let path = temp_store_path("unchanged");
        let compact = serde_json::to_string(
            &normalize_store_data(&json!({
                "version": STORE_VERSION,
                "active_id": "acct-1",
                "accounts": [account("acct-1")]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(&path, &compact).unwrap();

        let (store, ()) = mutate_store_at(&path, |store| {
            store["active_id"] = Value::String("acct-1".to_string());
            Ok(())
        })
        .unwrap();

        assert_eq!(store["active_id"], "acct-1");
        // The pretty-printed rewrite would differ byte for byte from the compact fixture.
        assert_eq!(fs::read_to_string(&path).unwrap(), compact);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn failed_update_leaves_store_untouched() {
        let path = temp_store_path("failed");
        mutate_store_at(&path, |store| {
            store["accounts"]
                .as_array_mut()
                .unwrap()
                .push(account("acct-1"));
            Ok(())
        })
        .unwrap();
        let before = fs::read_to_string(&path).unwrap();

        let err = mutate_store_at(&path, |store| -> Result<(), String> {
            store["accounts"] = json!([]);
            Err("fixture failure".to_string())
        })
        .unwrap_err();

        assert_eq!(err, "fixture failure");
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
