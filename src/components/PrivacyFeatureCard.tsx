interface PrivacyFeatureCardProps {
  title: string;
  description: string;
}

const PrivacyFeatureCard = ({ title, description }: PrivacyFeatureCardProps) => {
  return (
    <div className="bg-stone-800/50 rounded-xl p-3 border border-stone-700">
      <div className="flex items-start space-x-4">
        <div>
          <h3 className="font-semibold text-sm mb-2 text-center">{title}</h3>
          <p className="opacity-70 text-xs leading-relaxed text-center">{description}</p>
        </div>
      </div>
    </div>
  );
};

export default PrivacyFeatureCard;
