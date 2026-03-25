import OAuthProviderButton from '../components/oauth/OAuthProviderButton';
import { getProviderConfig } from '../components/oauth/providerConfigs';
import RotatingTetrahedronCanvas from '../components/RotatingTetrahedronCanvas';

const googleProvider = getProviderConfig('google');

const Welcome = () => {
  return (
    <div className="min-h-full w-full bg-[#090b12] px-4 py-6 md:px-10 md:py-10">
      <div className="mx-auto grid w-full grid-rows-[1fr_auto]   text-white">
        <section className="relative grid place-items-center px-6 py-14">
          <div className="relative z-10 flex w-full flex-col items-center gap-7 text-center">
            <div className="size-[150px] shrink-0">
              <RotatingTetrahedronCanvas />
            </div>

            <h1 className="text-balance text-4xl font-semibold tracking-tight text-white md:text-6xl">
              OpenHuman
            </h1>

            <p className="text-sm text-[#8e96b8] md:text-base">
              Your AI superhuman for personal and business life.
            </p>

            <div className="flex w-full max-w-sm flex-col items-stretch">
              {googleProvider ? <OAuthProviderButton provider={googleProvider} /> : null}
            </div>
          </div>
        </section>
      </div>
    </div>
  );
};

export default Welcome;
