import AccountsPage from './AccountsPage';
import ApiModePage from './ApiModePage';
import CodexPage from './CodexPage';
import SessionManagerPage, { useSessionManagerState } from './SessionManagerPage';
import SettingsPage from './SettingsPage';

export default function AppMainView({
  accountsPageProps,
  apiModePageProps,
  codexPageProps,
  sessionManagerPageProps,
  settingsPageProps,
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

  if (viewMode === 'sessions') {
    return <SessionManagerPage {...sessionManagerPageProps} sessionState={sessionManagerState} />;
  }

  return <AccountsPage {...accountsPageProps} />;
}
