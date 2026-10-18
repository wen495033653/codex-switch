import { useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { useI18n } from '../i18n';

const MENU_ITEM_HEIGHT = 32;
const MENU_PADDING = 6;
const MENU_GAP = 6;

// A card's less frequent actions behind a "…" button. The menu goes on <body>: the card clips
// its overflow, and its hover transform would carry a fixed child along with it.
export default function CardMoreMenu({ items }) {
  const { t } = useI18n();
  const buttonRef = useRef(null);
  const menuRef = useRef(null);
  const [position, setPosition] = useState(null);

  useEffect(() => {
    if (!position) return undefined;
    const close = () => setPosition(null);
    const closeOnOutsidePress = event => {
      if (menuRef.current && menuRef.current.contains(event.target)) return;
      if (buttonRef.current && buttonRef.current.contains(event.target)) return;
      close();
    };
    const closeOnEscape = event => {
      if (event.key === 'Escape') close();
    };
    window.addEventListener('pointerdown', closeOnOutsidePress);
    window.addEventListener('keydown', closeOnEscape);
    window.addEventListener('resize', close);
    window.addEventListener('scroll', close, true);
    return () => {
      window.removeEventListener('pointerdown', closeOnOutsidePress);
      window.removeEventListener('keydown', closeOnEscape);
      window.removeEventListener('resize', close);
      window.removeEventListener('scroll', close, true);
    };
  }, [position]);

  const toggle = () => {
    if (position) {
      setPosition(null);
      return;
    }
    const rect = buttonRef.current.getBoundingClientRect();
    const height = items.length * MENU_ITEM_HEIGHT + MENU_PADDING * 2 + 2;
    const fitsBelow = rect.bottom + MENU_GAP + height <= window.innerHeight;
    setPosition({
      right: window.innerWidth - rect.right,
      ...(fitsBelow
        ? { top: rect.bottom + MENU_GAP }
        : { bottom: window.innerHeight - rect.top + MENU_GAP })
    });
  };

  return (
    <>
      <button
        ref={buttonRef}
        type="button"
        className="icon-btn"
        title={t('更多操作')}
        aria-label={t('更多操作')}
        aria-haspopup="menu"
        aria-expanded={Boolean(position)}
        onClick={toggle}
      >
        <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
          <circle cx="5" cy="12" r="1.8" />
          <circle cx="12" cy="12" r="1.8" />
          <circle cx="19" cy="12" r="1.8" />
        </svg>
      </button>
      {position && createPortal(
        <div ref={menuRef} className="card-more-menu" role="menu" style={position}>
          {items.map(item => (
            <button
              key={item.key}
              type="button"
              role="menuitem"
              className={item.danger ? 'danger' : ''}
              disabled={item.disabled}
              title={item.title || undefined}
              onClick={() => {
                setPosition(null);
                item.onSelect();
              }}
            >
              {item.label}
            </button>
          ))}
        </div>,
        document.body
      )}
    </>
  );
}
