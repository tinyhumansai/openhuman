import DownloadScreen from '../components/DownloadScreen';
import TelegramLoginButton from '../components/TelegramLoginButton';
import TypewriterGreeting from '../components/TypewriterGreeting';

interface WelcomeProps {
  isWeb: boolean;
}

const Welcome = ({ isWeb }: WelcomeProps) => {
  const greetings = [
    'Hello HAL9000! 👋',
    "Let's cook! 🔥",
    'The A-Team is here! 👊',
    // "Welcome to the exclusive club of crypto degenerates! 🎪🚀",
    // "Let's get you richer than a Nigerian prince's email! 👑💸",
    // "Ready to HODL like your life depends on it? 🤝💀",
    // "Welcome, future crypto millionaire (results not guaranteed)! 🎰💎",
    // "Time to make Wall Street bros jealous AF! 📈🔥",
    // "Ready to go to the moon? Pack light! 🌙🚀"
  ];

  return (
    <div className="min-h-screen relative flex items-center justify-center">
      {/* Main content */}
      <div className="relative z-10 max-w-md w-full mx-4 space-y-6">
        {/* Welcome card */}
        <div className="glass rounded-3xl p-8 text-center animate-fade-up shadow-large">
          {/* Greeting */}
          <TypewriterGreeting greetings={greetings} />

          {/* <br /> */}

          <p className="opacity-70 mb-8 leading-relaxed">
            Welcome to AlphaHuman. Your Telegram assistant here to get you 10x more done in your
            journey.
          </p>

          <p className="opacity-70 leading-relaxed">Are you ready for this?</p>

          {/* Show Telegram login button in Tauri app, download screen on web */}
          {!isWeb && (
            <div className="mt-6">
              <TelegramLoginButton />
            </div>
          )}
        </div>

        {isWeb && <DownloadScreen />}

        {/* Bottom text */}
        <p className="text-center opacity-60 text-sm">Made with ❤️ by a bunch of Web3 nerds</p>
      </div>
    </div>
  );
};

export default Welcome;
