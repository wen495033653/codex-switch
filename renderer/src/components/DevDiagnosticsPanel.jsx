import { useState } from 'react';

export default function DevDiagnosticsPanel({
  entries,
  errorCount,
  isOpen,
  onClear,
  onToggle,
  warningCount
}) {
  const [isExpanded, setIsExpanded] = useState(false);
  const summaryText = `${entries.length} 条 / ${errorCount} 错误 / ${warningCount} 警告`;

  if (!isOpen) {
    return (
      <button
        type="button"
        className={`dev-diagnostics-toggle ${errorCount > 0 ? 'has-errors' : ''}`}
        onClick={onToggle}
      >
        <span>开发日志</span>
        <strong>{errorCount > 0 ? `错误 ${errorCount}` : entries.length}</strong>
      </button>
    );
  }

  return (
    <section
      className={`dev-diagnostics-panel ${isExpanded ? 'expanded' : ''}`}
      aria-label="开发日志"
    >
      <div className="dev-diagnostics-header">
        <div className="dev-diagnostics-title">
          <strong>开发日志</strong>
          <span>{summaryText}</span>
        </div>
        <div className="dev-diagnostics-actions">
          <button
            type="button"
            aria-pressed={isExpanded}
            onClick={() => setIsExpanded(prev => !prev)}
          >
            {isExpanded ? '缩小' : '放大'}
          </button>
          <button type="button" onClick={onClear}>清空</button>
          <button type="button" onClick={onToggle}>隐藏</button>
        </div>
      </div>

      <div className="dev-diagnostics-list">
        {entries.length === 0 ? (
          <div className="dev-diagnostics-empty">暂无日志</div>
        ) : entries.map(entry => (
          <article key={entry.id} className={`dev-diagnostics-entry ${entry.level}`}>
            <div className="dev-diagnostics-entry-meta">
              <span className="dev-diagnostics-entry-level">{entry.level}</span>
              <span>{entry.time}</span>
              <span>{entry.source}</span>
            </div>
            <pre>{entry.message}</pre>
          </article>
        ))}
      </div>
    </section>
  );
}
