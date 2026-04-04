import OAuthLoginSection from '../components/oauth/OAuthLoginSection';
import RotatingTetrahedronCanvas from '../components/RotatingTetrahedronCanvas';
import TypewriterGreeting from '../components/TypewriterGreeting';

const Welcome = () => {
  return (
    <div className="min-h-full bg-[#F5F5F5] flex items-center justify-center p-4 pt-6">
      <div className="flex w-full max-w-md flex-col items-center gap-7 text-center animate-fade-up">
        <div className="h-36 w-36 md:h-44 md:w-44">
          <RotatingTetrahedronCanvas />
        </div>

        <TypewriterGreeting
          greetings={['Hello HAL9000! 👋', "Let's cook! 🔥", 'The A-Team is here! 👊']}
        />

        <p className="max-w-xl text-sm text-stone-500 md:text-base">
          Welcome to <span className="font-medium text-stone-900">OpenHuman</span>! Your Personal AI
          super intelligence. Private, Simple and extremely powerful.
        </p>

        {/* <div className="glass rounded-3xl p-8 text-center animate-fade-up shadow-large"> */}
        <OAuthLoginSection />
        {/* </div> */}
      </div>
    </div>
  );
};

export default Welcome;
