import AccountsPage from './AccountsPage';
import ApiModePage from './ApiModePage';
import SessionManagerPage from './SessionManagerPage';
import SettingsPage from './SettingsPage';

export default function AppMainView({
  accountsPageProps,
  apiModePageProps,
  sessionManagerPageProps,
  settingsPageProps,
  viewMode
}) {
  if (viewMode === 'settings') {
    return <SettingsPage {...settingsPageProps} />;
  }

  if (viewMode === 'api') {
    return <ApiModePage {...apiModePageProps} />;
  }

  if (viewMode === 'sessions') {
    return <SessionManagerPage {...sessionManagerPageProps} />;
  }

  return <AccountsPage {...accountsPageProps} />;
}
