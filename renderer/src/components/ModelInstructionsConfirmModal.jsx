import Modal from './Modal';
import { useI18n } from '../i18n';

export default function ModelInstructionsConfirmModal({
    path,
    exists,
    backupPath,
    loading,
    onConfirm,
    onCancel
}) {
    const { t } = useI18n();
    const resolvedBackupPath = backupPath || (path ? `${path}.bak` : '');

    return (
        <Modal title={t('确认替换 GPT 破甲？')} onClose={onCancel} width="560px">
            {exists ? (
                <>
                    <p>{t('检测到已有 instruction.md，原有的 instruction.md 会改名成后面加 .bak 的备份文件，然后放入内置 GPT 破甲。')}</p>
                    <p className="model-instructions-file-path">{path}</p>
                    {resolvedBackupPath && (
                        <p className="model-instructions-file-path">{resolvedBackupPath}</p>
                    )}
                </>
            ) : (
                <>
                    <p>{t('将写入内置 GPT 破甲文件并在 config.toml 中启用。')}</p>
                    <p className="model-instructions-file-path">{path}</p>
                </>
            )}
            <div className="model-instructions-confirm-actions">
                <button type="button" className="btn btn-secondary" disabled={loading} onClick={onCancel}>{t('取消')}</button>
                <button type="button" className="btn btn-primary" autoFocus disabled={loading} onClick={onConfirm}>
                    {loading ? t('处理中...') : t('确认替换')}
                </button>
            </div>
        </Modal>
    );
}
