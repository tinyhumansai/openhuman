import { createContext, useContext } from 'react';

/**
 * Marks panels as being rendered inside the two-pane settings shell so shared
 * chrome (SettingsHeader) can adapt: on wide viewports the sidebar provides
 * navigation, so top-level panels hide their back button.
 *
 * Defaults to false so panels rendered outside the shell (tests, embedded
 * uses) keep their standalone behavior.
 */
const SettingsLayoutContext = createContext<{
  inTwoPaneShell: boolean;
  /**
   * Suppress the panel's own title/description/sub-nav chrome — used when a host
   * (e.g. the Connections pane) already renders a page header above the panel,
   * so the panel body renders headerless to avoid a doubled header.
   */
  headerless?: boolean;
}>({ inTwoPaneShell: false });

export const SettingsLayoutProvider = SettingsLayoutContext.Provider;

export const useSettingsLayout = () => useContext(SettingsLayoutContext);

export default SettingsLayoutContext;
