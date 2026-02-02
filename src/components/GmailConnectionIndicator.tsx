import GmailIcon from '../assets/icons/GmailIcon';

interface GmailConnectionIndicatorProps {
  description?: string;
  className?: string;
}

const GmailConnectionIndicator = ({
  description,
  className = '',
}: GmailConnectionIndicatorProps) => {
  // Gmail is always offline for now (placeholder)
  const gmailIsOnline = false;

  return (
    <div className={`mb-6 ${className}`}>
      <div className="flex items-center justify-center space-x-2 mb-3">
        <div
          className={`w-2 h-2 ${gmailIsOnline ? 'bg-red-500' : 'bg-gray-500'} rounded-full ${gmailIsOnline ? 'animate-pulse' : ''}`}></div>
        <div className="flex items-center space-x-1.5">
          <GmailIcon className={`w-4 h-4 ${gmailIsOnline ? 'text-red-500' : 'text-gray-500'}`} />
          <span className={`text-sm ${gmailIsOnline ? 'text-red-500' : 'text-gray-500'}`}>
            {gmailIsOnline ? 'Connected to Gmail' : 'Gmail is Offline'}
          </span>
        </div>
      </div>
      {description && (
        <p className="text-xs opacity-60 text-center leading-relaxed">{description}</p>
      )}
    </div>
  );
};

export default GmailConnectionIndicator;
