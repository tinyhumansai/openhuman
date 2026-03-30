interface RouteLoadingScreenProps {
  label?: string;
}

const RouteLoadingScreen = ({ label = 'Initializing OpenHuman...' }: RouteLoadingScreenProps) => {
  return (
    <div className="h-full min-h-[280px] w-full flex items-center justify-center">
      <div className="rounded-xl border border-white/10 bg-black/30 px-4 py-3 text-sm text-white/80">
        {label}
      </div>
    </div>
  );
};

export default RouteLoadingScreen;
