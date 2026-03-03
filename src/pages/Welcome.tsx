import DownloadScreen from '../components/DownloadScreen';
import OAuthLoginSection from '../components/oauth/OAuthLoginSection';
import TypewriterGreeting from '../components/TypewriterGreeting';

interface WelcomeProps {
  isWeb: boolean;
}

const Welcome = ({ isWeb }: WelcomeProps) => {
  const greetings = ['Hello HAL9000! 👋', "Let's cook! 🔥", 'The A-Team is here! 👊'];

  return (
    <div className="min-h-full relative flex items-center justify-center">
      {/* Main content */}
      <div className="relative z-10 max-w-md w-full mx-4 space-y-6">
        {/* Welcome card */}
        <div className="glass rounded-3xl p-8 text-center animate-fade-up shadow-large">
          {/* Greeting */}
          <TypewriterGreeting greetings={greetings} />

          <p className="opacity-70 mb-8 leading-relaxed">
            Welcome to AlphaHuman. Your Telegram assistant here to get you 10x more done in your
            journey.
          </p>

          <p className="opacity-70 leading-relaxed">Are you ready for this?</p>

          {/* Show OAuth login options in Tauri app, download screen on web */}
          {!isWeb && (
            <div className="mt-6">
              <OAuthLoginSection />
            </div>
          )}
        </div>

        {isWeb && <DownloadScreen />}
      </div>
    </div>
  );
};

export default Welcome;
