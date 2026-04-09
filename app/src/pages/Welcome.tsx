import OAuthProviderButton from '../components/oauth/OAuthProviderButton';
import { oauthProviderConfigs } from '../components/oauth/providerConfigs';
import RotatingTetrahedronCanvas from '../components/RotatingTetrahedronCanvas';

const Welcome = () => {
  return (
    <div className="min-h-full flex flex-col items-center justify-center p-4">
      <div className="max-w-md w-full">
        <div className="bg-white rounded-2xl shadow-soft border border-stone-200 p-8 animate-fade-up">
          {/* Logo */}
          <div className="flex justify-center mb-6">
            <div className="h-20 w-20">
              <RotatingTetrahedronCanvas />
            </div>
          </div>

          {/* Heading */}
          <h1 className="text-2xl font-bold text-stone-900 text-center mb-2">
            Sign in! Let's Cook
          </h1>

          {/* Subtitle */}
          <p className="text-sm text-stone-500 text-center mb-6 leading-relaxed">
            Welcome to <span className="font-medium text-stone-900">OpenHuman</span>! Your Personal
            AI Super Intelligence. Private, Simple and extremely powerful.
          </p>

          {/* OAuth buttons — horizontal row */}
          <div className="flex items-center justify-center gap-3 mb-5">
            {oauthProviderConfigs
              .filter(p => ['google', 'github', 'twitter'].includes(p.id))
              .map(provider => (
                <OAuthProviderButton
                  key={provider.id}
                  provider={provider}
                  className="!rounded-full !px-4 !py-2"
                />
              ))}
          </div>

          {/* Email login — disabled until backend auth flow is implemented
          <div className="flex items-center gap-3 mb-5">
            <div className="flex-1 h-px bg-stone-200" />
            <span className="text-xs text-stone-400">Or</span>
            <div className="flex-1 h-px bg-stone-200" />
          </div>
          <form className="space-y-3">
            <input
              type="email"
              placeholder="Enter your email"
              className="w-full rounded-xl border border-stone-200 bg-white px-4 py-3 text-sm text-stone-900 placeholder:text-stone-400 outline-none focus:border-primary-500 focus:ring-1 focus:ring-primary-500 transition-colors"
            />
            <button
              type="submit"
              className="w-full py-3 bg-primary-500 hover:bg-primary-600 text-white font-medium text-sm rounded-xl transition-colors duration-200">
              Continue with email
            </button>
          </form>
          */}
        </div>
      </div>
    </div>
  );
};

export default Welcome;
