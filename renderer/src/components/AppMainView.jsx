import { useState } from 'react';
import AccountsPage from './AccountsPage';
import ApiModePage from './ApiModePage';
import CodexPage from './CodexPage';
import { useIntelligenceTests } from '../hooks';
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
  // Here so a running 降智 test and the precheck dialog's tab outlive a page switch.
  const apiIntelligence = useIntelligenceTests();
  const subscriptionIntelligence = useIntelligenceTests();
  const [apiCheckTab, setApiCheckTab] = useState('precheck');

  if (viewMode === 'settings') {
    return <SettingsPage {...settingsPageProps} />;
  }

  if (viewMode === 'api') {
    return (
      <ApiModePage
        {...apiModePageProps}
        checkTab={apiCheckTab}
        intelligence={apiIntelligence}
        onCheckTabChange={setApiCheckTab}
      />
    );
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

  return <AccountsPage {...accountsPageProps} intelligence={subscriptionIntelligence} />;
}
