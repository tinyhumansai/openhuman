export interface SettingsEmptyStateProps {
  label: string;
  className?: string;
}

const SettingsEmptyState = ({ label, className }: SettingsEmptyStateProps) => {
  const classes = ['px-4 py-4 text-xs text-content-faint italic', className ?? '']
    .filter(Boolean)
    .join(' ');

  return <p className={classes}>{label}</p>;
};

export default SettingsEmptyState;
