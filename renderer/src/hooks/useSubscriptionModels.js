import { useEffect, useState } from 'react';
import { getRawErrorMessage } from '../utils/errors';

const EMPTY = { loading: false, models: [], clientVersion: '', error: '' };
const LOADING = { ...EMPTY, loading: true };

// The models a subscription account's Codex lists, read each time its test dialog opens.
export function useSubscriptionModels(accountId) {
  const [list, setList] = useState(null);

  useEffect(() => {
    if (!accountId) {
      setList(null);
      return undefined;
    }
    let current = true;
    window.api.listSubscriptionModels({ profileId: accountId })
      .then(result => {
        if (!result.ok) console.error('[list_subscription_models] failed', result);
        if (current) setList({ ...EMPTY, ...result, accountId });
      })
      .catch(err => {
        console.error('[list_subscription_models] failed', err);
        if (current) setList({ ...EMPTY, accountId, error: getRawErrorMessage(err) || '模型列表获取失败' });
      });
    return () => {
      current = false;
    };
  }, [accountId]);

  if (!accountId) return EMPTY;
  return list?.accountId === accountId ? list : LOADING;
}
