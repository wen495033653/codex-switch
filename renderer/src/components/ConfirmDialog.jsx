import { useId } from 'react';
import { useI18n } from '../i18n';

export default function ConfirmDialog({
    title,
    message,
    content = null,
    onConfirm,
    onCancel,
    isLoading,
    confirmText,
    loadingText,
    cancelText,
    width = '380px',
    confirmVariant = 'primary',
    showCancel = true
}) {
    const { t, translateRuntimeText } = useI18n();
    const isDanger = confirmVariant === 'danger';
    const widthClass = width === '460px' ? 'modal-content-lg' : 'modal-content-sm';
    const titleId = useId();
    const messageId = useId();
    const resolvedConfirmText = confirmText || t('确认');
    const resolvedLoadingText = loadingText || t('处理中...');
    const resolvedCancelText = cancelText || t('取消');

    return (
        <div className="modal-overlay">
            <div
                className={`modal-content confirm-dialog ${widthClass} ${isDanger ? 'confirm-dialog-danger' : 'confirm-dialog-primary'}`}
                role="dialog"
                aria-modal="true"
                aria-label={title ? undefined : t('确认')}
                aria-labelledby={title ? titleId : undefined}
                aria-describedby={!content && message ? messageId : undefined}
            >
                <div className="confirm-dialog-icon">
                    <svg fill="none" viewBox="0 0 24 24" stroke="currentColor">
                        <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 9v2m0 4h.01m-6.938 4h13.856c1.54 0 2.502-1.667 1.732-3L13.732 4c-.77-1.333-2.694-1.333-3.464 0L3.34 16c-.77 1.333.192 3 1.732 3z" />
                    </svg>
                </div>
                {title && <h3 className="confirm-dialog-title" id={titleId}>{translateRuntimeText(title)}</h3>}
                {content ? (
                    <div className="confirm-dialog-content">{content}</div>
                ) : (
                    <p className="confirm-dialog-message" id={messageId}>{translateRuntimeText(message)}</p>
                )}
                <div className="confirm-dialog-actions">
                    {showCancel && (
                        <button type="button" className="btn btn-secondary confirm-dialog-button" onClick={onCancel} disabled={isLoading}>
                            {resolvedCancelText}
                        </button>
                    )}
                    <button type="button" className="btn btn-primary confirm-dialog-button confirm-dialog-confirm" onClick={onConfirm} disabled={isLoading}>
                        {isLoading ? resolvedLoadingText : resolvedConfirmText}
                    </button>
                </div>
            </div>
        </div>
    );
}
