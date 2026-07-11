import { translateRuntimeText } from '../i18n';

export const getErrorMessage = (err, fallback) => {
  if (typeof err === 'string' && err.trim()) return translateRuntimeText(err.trim());
  if (err && typeof err.message === 'string' && err.message.trim()) return translateRuntimeText(err.message.trim());
  if (err && typeof err.error === 'string' && err.error.trim()) return translateRuntimeText(err.error.trim());
  return translateRuntimeText(fallback);
};
