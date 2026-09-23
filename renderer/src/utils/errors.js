import { translateRuntimeText } from '../i18n';

// The error text as the backend sent it. Compare against this, not against
// getErrorMessage(), which is already translated into the interface language.
export const getRawErrorMessage = (err) => {
  if (typeof err === 'string' && err.trim()) return err.trim();
  if (err && typeof err.message === 'string' && err.message.trim()) return err.message.trim();
  if (err && typeof err.error === 'string' && err.error.trim()) return err.error.trim();
  return '';
};

export const getErrorMessage = (err, fallback) => translateRuntimeText(getRawErrorMessage(err) || fallback);
