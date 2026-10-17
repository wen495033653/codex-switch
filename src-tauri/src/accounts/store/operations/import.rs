use super::{
    super::{
        model::{normalize_account, profile_id_from_account, sort_accounts_by_last_used},
        persistence::mutate_store,
    },
    mutation::{put_account, store_accounts_mut},
};
use serde_json::Value;

/// Adds the imported accounts the way a single import adds one: an account already stored is
/// replaced but keeps its timestamps and subscription snapshot. Returns the store and the
/// profile ids of the imported accounts.
pub(crate) fn import_store_accounts(
    incoming_accounts: Vec<Value>,
) -> Result<(Value, Vec<String>), String> {
    let mut incoming = Vec::new();
    for account in incoming_accounts {
        let account = normalize_account(&account)?;
        incoming.push((profile_id_from_account(&account)?, account));
    }
    let profile_ids = incoming
        .iter()
        .map(|(profile_id, _)| profile_id.clone())
        .collect();

    mutate_store(|store| {
        let accounts = store_accounts_mut(store)?;
        for (profile_id, account) in incoming {
            put_account(accounts, account, &profile_id);
        }
        sort_accounts_by_last_used(accounts);
        Ok(())
    })
    .map(|(store, ())| (store, profile_ids))
}
