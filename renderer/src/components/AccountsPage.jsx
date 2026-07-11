import AccountCard from './AccountCard';
import { getAccountId } from '../utils/auth';
import { useI18n } from '../i18n';

const ACCOUNT_FILTERS = ['ALL', 'FREE', 'PLUS', 'TEAM', 'PRO'];

export default function AccountsPage({
    accountGridRef,
    counts,
    currentAccountId,
    currentItems,
    filter,
    maskAccountName,
    onAddAccount,
    onDeleteAccount,
    onExportAccounts,
    onFilterChange,
    onPageChange,
    onRefreshAccount,
    onRefreshAllClick,
    onSearchChange,
    onSwitchAccount,
    onOpenCodexAppInstance,
    openingCodexAppTarget,
    runningCodexAppInstances,
    onOpenUsageStatsDetail,
    onViewRefreshToken,
    page,
    pageSize,
    refreshAllStatus,
    refreshingAccountId,
    search,
    startIdx,
    switching,
    total,
    totalPages,
    usageStatsBySubscription
}) {
    const { t } = useI18n();

    return (
        <>
            <div className="toolbar account-toolbar">
                <div className="account-toolbar-row">
                    <div className="search-wrapper">
                        <svg className="search-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true">
                            <circle cx="11" cy="11" r="7" />
                            <path d="m20 20-3.5-3.5" />
                        </svg>
                        <input
                            className="search-input"
                            placeholder={t('搜索账号...')}
                            aria-label={t('搜索账号...')}
                            value={search}
                            onChange={e => onSearchChange(e.target.value)}
                        />
                    </div>
                    <div className="nav-tabs account-filter-tabs">
                        {ACCOUNT_FILTERS.map(item => (
                            <button
                                key={item}
                                type="button"
                                className={`nav-item ${filter === item ? 'active' : ''}`}
                                onClick={() => onFilterChange(item)}
                                aria-pressed={filter === item}
                            >
                                {item === 'ALL' ? t('全部') : item} <span className="account-filter-count">{counts[item]}</span>
                            </button>
                        ))}
                    </div>
                    <div className="account-toolbar-actions">
                        <button
                            className={`btn btn-secondary account-refresh-all-button ${refreshAllStatus.running ? 'is-running' : ''}`}
                            title={refreshAllStatus.running
                                ? t('后台刷新中（{completed}/{total}）', {
                                    completed: refreshAllStatus.completed,
                                    total: refreshAllStatus.total
                                })
                                : t('刷新所有配额')}
                            aria-label={refreshAllStatus.running ? t('查看配额刷新进度') : t('刷新所有配额')}
                            aria-busy={refreshAllStatus.running}
                            onClick={onRefreshAllClick}
                        >
                            <svg className={`toolbar-refresh-icon ${refreshAllStatus.running ? 'icon-spin' : ''}`} fill="none" viewBox="0 0 24 24" stroke="currentColor">
                                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                            </svg>
                            <span className="account-refresh-all-label">
                                {refreshAllStatus.running ? t('刷新中') : t('刷新配额')}
                            </span>
                        </button>
                        <button type="button" className="btn btn-secondary account-export-button" onClick={onExportAccounts}>
                            <svg className="toolbar-action-icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" aria-hidden="true">
                                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M12 3v12m0 0 4-4m-4 4-4-4M5 18v2h14v-2" />
                            </svg>
                            <span>{t('导出账号')}</span>
                        </button>
                        <button type="button" className="btn btn-primary" onClick={onAddAccount}>
                            <span className="btn-leading-icon">+</span>
                            <span>{t('添加账号')}</span>
                        </button>
                    </div>
                </div>
            </div>

            <div className="list-panel">
                <div className="account-grid" ref={accountGridRef}>
                    {currentItems.map(acc => {
                        const accountId = getAccountId(acc);
                        const usageStats = usageStatsBySubscription && accountId
                            ? usageStatsBySubscription[accountId]
                            : null;
                        return (
                            <AccountCard
                                key={accountId}
                                acc={acc}
                                isCurrent={accountId === currentAccountId}
                                refreshing={refreshAllStatus.running || refreshingAccountId === accountId}
                                switching={switching}
                                usageStats={usageStats}
                                maskAccountName={maskAccountName}
                                onSwitch={onSwitchAccount}
                                onOpenCodexAppInstance={onOpenCodexAppInstance}
                                openingCodexAppTarget={openingCodexAppTarget}
                                runningCodexAppInstances={runningCodexAppInstances}
                                onRefresh={onRefreshAccount}
                                onDelete={onDeleteAccount}
                                onViewRefreshToken={onViewRefreshToken}
                                onOpenUsageStatsDetail={onOpenUsageStatsDetail}
                            />
                        );
                    })}

                    {currentItems.length === 0 && (
                        <div className="empty-state empty-state-card">{t('暂无账号数据')}</div>
                    )}
                </div>

                <div className="panel-footer">
                    <div className="footer-info">
                        {t('显示第 {start} 到 {end} 条，共 {total} 条', {
                            start: total === 0 ? 0 : startIdx + 1,
                            end: Math.min(startIdx + pageSize, total),
                            total
                        })}
                    </div>
                    {totalPages > 0 && (
                        <div className="pagination">
                            <button type="button" className="page-btn" aria-label={t('上页')} disabled={page === 1} onClick={() => onPageChange(Math.max(1, page - 1))}>
                                &lt;
                            </button>
                            {Array.from({ length: totalPages }, (_, i) => i + 1).map(item => (
                                <button type="button" key={item} className={`page-btn ${page === item ? 'active' : ''}`} aria-current={page === item ? 'page' : undefined} onClick={() => onPageChange(item)}>
                                    {item}
                                </button>
                            ))}
                            <button type="button" className="page-btn" aria-label={t('下页')} disabled={page === totalPages} onClick={() => onPageChange(Math.min(totalPages, page + 1))}>
                                &gt;
                            </button>
                        </div>
                    )}
                </div>
            </div>
        </>
    );
}
