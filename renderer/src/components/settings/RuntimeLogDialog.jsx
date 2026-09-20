import { useState } from 'react';
import Modal from '../Modal';
import { useI18n } from '../../i18n';
import { useRuntimeLogs } from '../../hooks/useRuntimeLogs';

const LEVELS = [['all', '全部'], ['success', '成功'], ['warn', '警告'], ['error', '错误']];

export default function RuntimeLogDialog({ onClose }) {
    const logs = useRuntimeLogs();
    return <RuntimeLogView {...logs} onClose={onClose} />;
}

export function RuntimeLogView({ entries, loading, error, writeError, refresh, onClose }) {
    const { t, language } = useI18n();
    const [filter, setFilter] = useState('all');
    const visible = entries.filter(entry => filter === 'all' || entry.level === filter);

    return (
        <Modal title={t('Codex 运行日志')} onClose={onClose} width="760px">
            <div className="runtime-log-body" onKeyDown={event => {
                if (event.key === 'Escape') { event.stopPropagation(); onClose(); }
            }}>
                <p className="runtime-log-hint">{t('最近 500 条，时间倒序。只记录运行结果，不展示调试噪声。')}</p>
                <div className="runtime-log-toolbar">
                    <div className="runtime-log-filters" role="group" aria-label={t('日志级别')}>
                        {LEVELS.map(([level, label]) => (
                            <button key={level} type="button" className={`runtime-log-filter ${filter === level ? 'active' : ''}`}
                                aria-pressed={filter === level} onClick={() => setFilter(level)}>
                                {t(label)} <span>{level === 'all' ? entries.length : entries.filter(entry => entry.level === level).length}</span>
                            </button>
                        ))}
                    </div>
                    <button type="button" className="btn btn-secondary" onClick={refresh} disabled={loading}>
                        {loading ? t('读取中...') : t('刷新')}
                    </button>
                </div>
                {error && <p className="runtime-log-error" role="alert">{t('读取日志失败')}：{error}</p>}
                {writeError && <p className="runtime-log-error" role="alert">{t('部分日志未能保存')}：{writeError}</p>}
                <div className="runtime-log-list" aria-busy={loading}>
                    {!loading && !error && visible.length === 0 && (
                        <p className="runtime-log-empty">{t(entries.length ? '此级别暂无日志' : '暂无日志')}</p>
                    )}
                    {visible.map(entry => (
                        <article className={`runtime-log-entry ${entry.level}`} key={entry.id}>
                            <div className="runtime-log-entry-header">
                                <span className={`runtime-log-level ${entry.level}`}>{t(LEVELS.find(([level]) => level === entry.level)?.[1] || '错误')}</span>
                                <strong>{t(entry.title)}</strong>
                                <time dateTime={entry.timestamp}>{new Date(entry.timestamp).toLocaleString(language === 'en' ? 'en-GB' : 'zh-CN', { hour12: false })}</time>
                            </div>
                            <p className="runtime-log-summary">{entry.summary}</p>
                            {entry.action && <p className="runtime-log-action">{entry.action}</p>}
                            <details className="runtime-log-details">
                                <summary>{t('详情')}</summary>
                                <pre className="runtime-log-raw"><code>{entry.rawLog}</code></pre>
                            </details>
                        </article>
                    ))}
                </div>
                <div className="runtime-log-footer">
                    <span>{t('日志保存在本机，重开 Codex Switch 后仍可查看。')}</span>
                    <button type="button" className="btn btn-secondary" autoFocus onClick={onClose}>{t('关闭')}</button>
                </div>
            </div>
        </Modal>
    );
}
