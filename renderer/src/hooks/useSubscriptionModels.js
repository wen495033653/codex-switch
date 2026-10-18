import { useEffect, useState } from 'react';
import { getErrorMessage } from '../utils/errors';

const IDLE = { loading: false, models: [], clientVersion: '', error: '' };

// The models a subscription account's Codex lists, read each time its test dialog opens.
export function useSubscriptionModels(accountId) {
  const [list, setList] = useState(IDLE);

  useEffect(() => {
    if (!accountId) {
      setList(IDLE);
      return undefined;
    }
    let current = true;
    setList({ ...IDLE, loading: true });
    window.api.listSubscriptionModels({ profileId: accountId })
      .then(result => {
        if (current) setList({ ...IDLE, models: result.models, clientVersion: result.clientVersion });
      })
      .catch(err => {
        console.error('[list_subscription_models] failed', err);
        if (current) setList({ ...IDLE, error: getErrorMessage(err, '模型列表获取失败') });
      });
    return () => {
      current = false;
    };
  }, [accountId]);

  return list;
}
