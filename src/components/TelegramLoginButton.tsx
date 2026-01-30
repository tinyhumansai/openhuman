import { TELEGRAM_BOT_USERNAME } from "../utils/config";
import { openUrl } from "../utils/openUrl";
import { isTauri } from "../utils/tauriCommands";


interface TelegramLoginButtonProps {
  className?: string;
  text?: string;
  disabled?: boolean;
}

const TelegramLoginButton = ({
  className = "",
  text = "Yes, Login with Telegram",
  disabled: externalDisabled = false,
}: TelegramLoginButtonProps) => {
  const handleTelegramLogin = async () => {
    if (externalDisabled) return;

    console.log("Starting Telegram login", isTauri());

    // Desktop (Tauri): use system browser → backend Telegram widget → deep link back to app.
    if (isTauri()) await openUrl(`https://t.me/${TELEGRAM_BOT_USERNAME}?start=login_desktop`);

    // Web fallback: open bot (existing flow).
    await openUrl(`https://t.me/${TELEGRAM_BOT_USERNAME}?start=login`);
  };

  const isDisabled = externalDisabled;

  return (
    <button
      onClick={handleTelegramLogin}
      disabled={isDisabled}
      className={`w-full flex items-center justify-center space-x-3 bg-blue-500 hover:bg-blue-600 active:bg-blue-700 disabled:bg-blue-400 disabled:cursor-not-allowed text-white font-semibold py-4 rounded-xl transition-all duration-300 hover:shadow-medium hover:scale-[1.02] active:scale-[0.98] disabled:hover:scale-100 ${className}`}
    >
      <svg className="w-6 h-6" viewBox="0 0 24 24" fill="currentColor">
        <path d="M11.944 0A12 12 0 0 0 0 12a12 12 0 0 0 12 12 12 12 0 0 0 12-12A12 12 0 0 0 12 0a12 12 0 0 0-.056 0zm4.962 7.224c.1-.002.321.023.465.14a.506.506 0 0 1 .171.325c.016.093.036.306.02.472-.18 1.898-.962 6.502-1.36 8.627-.168.9-.499 1.201-.82 1.23-.696.065-1.225-.46-1.9-.902-1.056-.693-1.653-1.124-2.678-1.8-1.185-.78-.417-1.21.258-1.91.177-.184 3.247-2.977 3.307-3.23.007-.032.014-.15-.056-.212s-.174-.041-.249-.024c-.106.024-1.793 1.14-5.061 3.345-.48.33-.913.49-1.302.48-.428-.008-1.252-.241-1.865-.44-.752-.245-1.349-.374-1.297-.789.027-.216.325-.437.893-.663 3.498-1.524 5.83-2.529 6.998-3.014 3.332-1.386 4.025-1.627 4.476-1.635z" />
      </svg>
      <span>{text}</span>
    </button>
  );
};

export default TelegramLoginButton;
