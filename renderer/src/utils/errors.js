export const getErrorMessage = (err, fallback) => {
  if (typeof err === 'string' && err.trim()) return err.trim();
  if (err && typeof err.message === 'string' && err.message.trim()) return err.message.trim();
  if (err && typeof err.error === 'string' && err.error.trim()) return err.error.trim();
  return fallback;
};
