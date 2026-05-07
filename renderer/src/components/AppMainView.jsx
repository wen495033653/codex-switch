import AccountsPage from './AccountsPage';
import ApiModePage from './ApiModePage';
import SettingsPage from './SettingsPage';

export default function AppMainView({
  accountsPageProps,
  apiModePageProps,
  settingsPageProps,
  viewMode
}) {
  if (viewMode === 'settings') {
    return <SettingsPage {...settingsPageProps} />;
  }

  if (viewMode === 'api') {
    return <ApiModePage {...apiModePageProps} />;
  }

  return <AccountsPage {...accountsPageProps} />;
}
