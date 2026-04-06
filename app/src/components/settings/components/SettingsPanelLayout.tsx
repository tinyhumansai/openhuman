import { ReactNode } from 'react';

import SettingsBackButton from './SettingsBackButton';

interface SettingsPanelLayoutProps {
  title: string;
  onBack: () => void;
  children: ReactNode;
  className?: string;
}

const SettingsPanelLayout = ({
  title,
  onBack,
  children,
  className = '',
}: SettingsPanelLayoutProps) => {
  return (
    <div className={`glass rounded-3xl overflow-hidden ${className}`}>
      <SettingsBackButton onClick={onBack} title={title} />
      <div>{children}</div>
    </div>
  );
};

export default SettingsPanelLayout;
