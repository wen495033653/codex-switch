import AccountsPage from './AccountsPage';
import ApiModePage from './ApiModePage';
import CodexPage from './CodexPage';
import { useSessionManagerState } from '../hooks/useSessionManagerState';
import SessionManagerPage from './SessionManagerPage';
import SettingsPage from './SettingsPage';
import UsageBillsPage from './UsageBillsPage';

export default function AppMainView({
  accountsPageProps,
  apiModePageProps,
  codexPageProps,
  sessionManagerPageProps,
  settingsPageProps,
  usageBillsPageProps,
  viewMode
}) {
  const sessionManagerState = useSessionManagerState();

  if (viewMode === 'settings') {
    return <SettingsPage {...settingsPageProps} />;
  }

  if (viewMode === 'api') {
    return <ApiModePage {...apiModePageProps} />;
  }

  if (viewMode === 'codex') {
    return <CodexPage {...codexPageProps} />;
  }

  if (viewMode === 'bills') {
    return <UsageBillsPage {...usageBillsPageProps} />;
  }

  if (viewMode === 'sessions') {
    return <SessionManagerPage {...sessionManagerPageProps} sessionState={sessionManagerState} />;
  }

  return <AccountsPage {...accountsPageProps} />;
}
