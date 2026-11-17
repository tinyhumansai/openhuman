import { useState } from 'react';

import AIPanel from '../../../components/settings/panels/AIPanel';
import WhatLeavesLink from '../../../features/privacy/WhatLeavesLink';
import { useT } from '../../../lib/i18n/I18nContext';
import CustomWizardConfigPage from './CustomWizardConfigPage';

/**
 * Step 1 — pick a model provider.
 *
 * `hideTabChrome` collapses the panel to its providers view. Without it the
 * wizard inherited the full Settings surface: provider/routing chip tabs, and
 * a full-height scroll region inside a card that has no fixed height.
 *
 * The panel keeps its own `SaveBar`, so the step tracks the panel's dirty
 * state and blocks Continue while edits are pending. Continue used to navigate
 * away and drop them with no warning.
 */
const CustomInferencePage = () => {
  const { t } = useT();
  const [dirty, setDirty] = useState(false);

  return (
    <CustomWizardConfigPage
      stepKey="inference"
      backRoute="/"
      continueDisabled={dirty}
      continueHint={dirty ? t('onboarding.custom.unsavedChanges') : undefined}
      configureContent={
        <div className="space-y-4">
          <AIPanel embedded hideTabChrome onDirtyChange={setDirty} />
          <div className="flex justify-center">
            <WhatLeavesLink />
          </div>
        </div>
      }
    />
  );
};

export default CustomInferencePage;
