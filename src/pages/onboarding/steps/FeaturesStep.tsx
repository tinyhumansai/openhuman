import PrivacyFeatureCard from '../../../components/PrivacyFeatureCard';

interface FeaturesStepProps {
  onNext: () => void;
}

const FeaturesStep = ({ onNext }: FeaturesStepProps) => {
  const features = [
    {
      title: 'Keep track of Everything',
      description:
        'Sometimes your chats, emails, tasks can get a bit too much. Stay on track, organize things and get more done.',
    },
    {
      title: 'Has Infinite Memory & Learns',
      description:
        'Missed something? Have a sexy assistant give you exactly what you need, every time.',
    },
    {
      title: 'Trades the Trenches',
      description:
        "With it's own private wallet, trade or reasearch on any exchange or shitcoin autonomously. Go big or go home.",
    },
  ];

  return (
    <div className="glass rounded-3xl p-8 shadow-large animate-fade-up">
      <div className="text-center mb-4">
        <h1 className="text-xl font-bold mb-2">Ready to get started?</h1>
        <p className="opacity-70 text-sm">Here's some of the things that OpenHuman can do</p>
      </div>

      <div className="space-y-2 mb-4">
        {features.map((feature, index) => (
          <PrivacyFeatureCard key={index} title={feature.title} description={feature.description} />
        ))}
      </div>

      <button onClick={onNext} className="btn-primary w-full py-2.5 text-sm font-medium rounded-xl">
        Looks Amazing. Bring It On 🚀
      </button>
    </div>
  );
};

export default FeaturesStep;
