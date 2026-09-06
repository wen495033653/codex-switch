import Modal from './Modal';
import { useI18n } from '../i18n';

export default function ModelInstructionsConfirmModal({ path, loading, onKeep, onOverwrite, onCancel }) {
    const { t } = useI18n();
    return (
        <Modal title={t('检测到本地提示词文件')} onClose={onCancel} width="560px">
            <p>{t('本地已有 gpt-unrestricted.md，是否用应用内置版本覆盖？')}</p>
            <p className="model-instructions-file-path">{path}</p>
            <p>{t('保留本地会使用你修改过的内容；覆盖前会在同目录备份原文件。')}</p>
            <div className="model-instructions-confirm-actions">
                <button type="button" className="btn btn-secondary" disabled={loading} onClick={onCancel}>{t('取消')}</button>
                <button type="button" className="btn btn-secondary" disabled={loading} onClick={onOverwrite}>{t('覆盖并启用')}</button>
                <button type="button" className="btn btn-primary" autoFocus disabled={loading} onClick={onKeep}>{t('保留本地并启用')}</button>
            </div>
        </Modal>
    );
}
