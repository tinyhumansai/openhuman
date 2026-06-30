import { useT } from '../lib/i18n/I18nContext';

interface RouteLoadingScreenProps {
  label?: string;
}

const RouteLoadingScreen = ({ label }: RouteLoadingScreenProps) => {
  const { t } = useT();
  return (
    <div className="h-full min-h-[280px] w-full flex items-center justify-center">
      <div className="rounded-xl border border-line bg-surface px-4 py-3 text-sm text-content-secondary">
        {label ?? t('app.routeLoading.initializing')}
      </div>
    </div>
  );
};

export default RouteLoadingScreen;
