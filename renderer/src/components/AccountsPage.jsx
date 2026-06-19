import AccountCard from './AccountCard';
import { getAccountId } from '../utils/auth';

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
    return (
        <>
            <div className="toolbar account-toolbar">
                <div className="account-toolbar-row">
                    <div className="search-wrapper">
                        <span className="search-icon">🔍</span>
                        <input
                            className="search-input"
                            placeholder="搜索账号..."
                            value={search}
                            onChange={e => onSearchChange(e.target.value)}
                        />
                    </div>
                    <div className="nav-tabs account-filter-tabs">
                        {ACCOUNT_FILTERS.map(item => (
                            <div
                                key={item}
                                className={`nav-item ${filter === item ? 'active' : ''}`}
                                onClick={() => onFilterChange(item)}
                            >
                                {item === 'ALL' ? '全部' : item} <span className="account-filter-count">{counts[item]}</span>
                            </div>
                        ))}
                    </div>
                    <div className="action-bar">
                        <button
                            className="btn btn-secondary"
                            onClick={onExportAccounts}
                        >
                            导出数据
                        </button>
                    </div>
                    <button className="btn btn-primary" onClick={onAddAccount}>
                        <span>+ 添加账号</span>
                    </button>
                    <button
                        className="btn btn-secondary btn-icon-only"
                        title={refreshAllStatus.running
                            ? `后台刷新中（${refreshAllStatus.completed}/${refreshAllStatus.total}）`
                            : '刷新所有配额'}
                        onClick={onRefreshAllClick}
                    >
                        <svg className={`toolbar-refresh-icon ${refreshAllStatus.running ? 'icon-spin' : ''}`} fill="none" viewBox="0 0 24 24" stroke="currentColor">
                            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M4 4v5h.582m15.356 2A8.001 8.001 0 004.582 9m0 0H9m11 11v-5h-.581m0 0a8.003 8.003 0 01-15.357-2m15.357 2H15" />
                        </svg>
                    </button>
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
                        <div className="empty-state empty-state-card">暂无账号数据</div>
                    )}
                </div>

                <div className="panel-footer">
                    <div className="footer-info">
                        显示第 {total === 0 ? 0 : startIdx + 1} 到 {Math.min(startIdx + pageSize, total)} 条，共 {total} 条
                    </div>
                    {totalPages > 0 && (
                        <div className="pagination">
                            <button className="page-btn" disabled={page === 1} onClick={() => onPageChange(Math.max(1, page - 1))}>
                                &lt;
                            </button>
                            {Array.from({ length: totalPages }, (_, i) => i + 1).map(item => (
                                <button key={item} className={`page-btn ${page === item ? 'active' : ''}`} onClick={() => onPageChange(item)}>
                                    {item}
                                </button>
                            ))}
                            <button className="page-btn" disabled={page === totalPages} onClick={() => onPageChange(Math.min(totalPages, page + 1))}>
                                &gt;
                            </button>
                        </div>
                    )}
                </div>
            </div>
        </>
    );
}
