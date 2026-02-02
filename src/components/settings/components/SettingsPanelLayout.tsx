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
    <div className={`glass rounded-3xl overflow-hidden h-[600px] flex flex-col ${className}`}>
      <SettingsBackButton onClick={onBack} title={title} />
      <div className="flex-1 overflow-y-auto">{children}</div>
    </div>
  );
};

export default SettingsPanelLayout;
