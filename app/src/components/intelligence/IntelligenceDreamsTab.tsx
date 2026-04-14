export default function IntelligenceDreamsTab() {
  return (
    <div className="glass rounded-2xl p-8 text-center animate-fade-up">
      <div className="w-16 h-16 mx-auto mb-4 flex items-center justify-center rounded-full bg-sky-500/10">
        <svg className="w-8 h-8 text-sky-400" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={1.5}
            d="M2.036 12.322a1.012 1.012 0 010-.639C3.423 7.51 7.36 4.5 12 4.5c4.638 0 8.573 3.007 9.963 7.178.07.207.07.431 0 .639C20.577 16.49 16.64 19.5 12 19.5c-4.638 0-8.573-3.007-9.963-7.178z"
          />
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={1.5}
            d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
          />
        </svg>
      </div>
      <h2 className="text-lg font-semibold text-stone-900 mb-2">Dreams</h2>
      <p className="text-stone-400 text-sm mb-1">
        Twice every day, OpenHuman will generate a dream (or a summary) based on
        everything that has happened in your life today. These dreams are then
        indexed and can be used to influence OpenHuman&apos;s behavior.
      </p>
      <p className="text-xs text-stone-500">Coming soon</p>
    </div>
  );
}
