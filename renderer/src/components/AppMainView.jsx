import AccountsPage from './AccountsPage';
import ApiModePage from './ApiModePage';
import SessionManagerPage, { useSessionManagerState } from './SessionManagerPage';
import SettingsPage from './SettingsPage';

export default function AppMainView({
  accountsPageProps,
  apiModePageProps,
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

  if (viewMode === 'sessions') {
    return <SessionManagerPage {...sessionManagerPageProps} sessionState={sessionManagerState} />;
  }

  return <AccountsPage {...accountsPageProps} />;
}
