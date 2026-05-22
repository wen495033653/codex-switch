import { useEffect, useRef, useState } from 'react';
import { getErrorMessage } from '../utils/errors';

export function useToast() {
  const [message, setMessage] = useState('');
  const timerRef = useRef(null);

  const toast = (msg, duration = 3000) => {
    if (timerRef.current) clearTimeout(timerRef.current);
    setMessage(msg);
    timerRef.current = setTimeout(() => {
      setMessage('');
      timerRef.current = null;
    }, duration);
  };

  const toastError = (err, fallback, duration = 3000) => {
    const message = getErrorMessage(err, fallback);
    if (import.meta.env.DEV) {
      console.error(message, err);
    }
    toast(message, duration);
  };

  useEffect(() => () => {
    if (timerRef.current) clearTimeout(timerRef.current);
  }, []);

  return { message, toast, toastError };
}
