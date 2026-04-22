import { useEffect, useState } from 'react';

import ProgressIndicator from '../../../components/ProgressIndicator';
import OnboardingNextButton from '../components/OnboardingNextButton';

interface WelcomeStepProps {
  onNext: () => void;
}

const TOTAL_SLIDES = 3;
const AUTO_ADVANCE_MS = 5000;

/* ------------------------------------------------------------------ */
/*  Slide 1 — Welcome                                                 */
/* ------------------------------------------------------------------ */
const WelcomeSlide = () => (
  <div className="flex flex-col items-center text-center">
    <img src="/logo.png" alt="OpenHuman" className="w-24 h-24 rounded-2xl mb-6" />
    <h1 className="text-2xl font-bold font-display text-stone-900 mb-3">Welcome On Board</h1>
    <p className="text-stone-500 text-sm leading-relaxed">
      All your tasks, tools, and conversations organized in one place.
    </p>
  </div>
);

/* ------------------------------------------------------------------ */
/*  Slide 2 — Integrations (image placeholder)                        */
/* ------------------------------------------------------------------ */
const IntegrationsSlide = () => (
  <div className="flex flex-col items-center text-center">
    <h1 className="text-2xl font-bold font-display text-stone-900 mb-3">
      Manage work without{'\n'}switching apps
    </h1>
    <p className="text-stone-500 text-sm leading-relaxed mb-6">
      Handle GitHub reviews, track Notion tasks, check Slack messages, manage your community and
      many more — all from a single Place
    </p>
    <img
      src="/onboarding-manage-work.png"
      alt="Integration icons showing connected apps"
      className="w-full max-h-36 object-contain"
    />
  </div>
);

/* ------------------------------------------------------------------ */
/*  Slide 3 — Automation (image placeholder)                           */
/* ------------------------------------------------------------------ */
const AutomationSlide = () => (
  <div className="flex flex-col items-center text-center">
    <h1 className="text-2xl font-bold font-display text-stone-900 mb-3">Automate it all</h1>
    <p className="text-stone-500 text-sm leading-relaxed mb-6">
      Save time by automating your daily workflow. Everything you need, right at your fingertips.
    </p>
    <img
      src="/onboarding-automate-all.png"
      alt="Automated tasks from Notion, Slack, and GitHub"
      className="w-full max-h-48 object-contain"
    />
  </div>
);

/* ------------------------------------------------------------------ */
/*  WelcomeStep — auto-advancing carousel, button goes to next step    */
/* ------------------------------------------------------------------ */
const WelcomeStep = ({ onNext }: WelcomeStepProps) => {
  const [slide, setSlide] = useState(0);

  useEffect(() => {
    const timer = setInterval(() => {
      setSlide(prev => (prev + 1) % TOTAL_SLIDES);
    }, AUTO_ADVANCE_MS);
    return () => clearInterval(timer);
  }, []);

  return (
    <div
      data-testid="onboarding-welcome-step"
      className="rounded-2xl bg-white p-10 shadow-soft animate-fade-up">
      <div className="h-[340px] flex flex-col items-center justify-center">
        {slide === 0 && <WelcomeSlide />}
        {slide === 1 && <IntegrationsSlide />}
        {slide === 2 && <AutomationSlide />}
      </div>
      <div className="mt-8 mb-6 flex justify-center">
        <ProgressIndicator currentStep={slide} totalSteps={TOTAL_SLIDES} />
      </div>
      <OnboardingNextButton label="Let's Start" onClick={onNext} />
    </div>
  );
};

export default WelcomeStep;
