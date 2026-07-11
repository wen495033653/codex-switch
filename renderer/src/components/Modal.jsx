import { useId } from 'react';

export default function Modal({ title, onClose, children, width = '440px' }) {
    const titleId = useId();
    const widthClass = width === '760px'
        ? 'modal-content-xxl'
        : (width === '560px' ? 'modal-content-xl' : 'modal-content-md');

    return (
        <div className="modal-overlay" onClick={e => e.target === e.currentTarget && onClose()}>
            <div className={`modal-content ${widthClass}`} role="dialog" aria-modal="true" aria-labelledby={titleId}>
                <h3 className="modal-title" id={titleId}>{title}</h3>
                {children}
            </div>
        </div>
    );
}
