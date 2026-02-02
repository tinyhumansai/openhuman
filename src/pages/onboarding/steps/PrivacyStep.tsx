import PrivacyFeatureCard from '../../../components/PrivacyFeatureCard';

interface PrivacyStepProps {
  onNext: () => void;
}

const PrivacyStep = ({ onNext }: PrivacyStepProps) => {
  const privacyFeatures = [
    {
      title: '🔒 Everything is Local & Encrypted',
      description:
        'Your data is encrypted (AES-256-GCM) in your device and never stored elsewhere. Your encryption keys never leave your device.',
    },
    {
      title: '🙈 Zero Data Retention',
      description:
        'Your queries are processed, immediately discarded and never stored elsewhere. Your data is NEVER used to train AI models. ',
    },
    {
      title: '🔥 Delete Anytime You Want',
      description:
        'You can delete your data and your account anytime you want. Everything will get wiped including AI memories.',
    },
  ];

  return (
    <div className="glass rounded-3xl p-8 shadow-large animate-fade-up">
      <div className="text-center mb-4">
        <h1 className="text-xl font-bold mb-2">A Quick Privacy Note</h1>
        <p className="opacity-70 text-sm">
          Since AlphaHuman handles criticial information about you, here's how it handles your data
          and manages your privacy.
        </p>
      </div>

      <div className="space-y-2 mb-4">
        {privacyFeatures.map((feature, index) => (
          <PrivacyFeatureCard key={index} title={feature.title} description={feature.description} />
        ))}
      </div>

      <button onClick={onNext} className="btn-primary w-full py-2.5 text-sm font-medium rounded-xl">
        Got it! Let's Continue 👀
      </button>
    </div>
  );
};

export default PrivacyStep;
