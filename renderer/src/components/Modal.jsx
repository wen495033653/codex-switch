export default function Modal({ title, onClose, children, width = '440px' }) {
    const widthClass = width === '560px' ? 'modal-content-xl' : 'modal-content-md';

    return (
        <div className="modal-overlay" onClick={e => e.target === e.currentTarget && onClose()}>
            <div className={`modal-content ${widthClass}`}>
                <h3 className="modal-title">{title}</h3>
                {children}
            </div>
        </div>
    );
}
