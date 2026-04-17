import { FaLinkedin } from 'react-icons/fa';
import { SiDiscord, SiGmail, SiGooglemeet, SiSlack, SiTelegram, SiWhatsapp } from 'react-icons/si';
import { TbRobot } from 'react-icons/tb';

import type { AccountProvider } from '../../types/accounts';

/**
 * Brand colors for the provider icons — matches each service's own
 * marketing identity. Kept in one place so they stay consistent wherever
 * the icon is reused (sidebar rail, add-account modal, etc.).
 */
const PROVIDER_COLOR: Record<AccountProvider, string> = {
  whatsapp: '#25D366',
  telegram: '#229ED9',
  linkedin: '#0A66C2',
  gmail: '#EA4335',
  slack: '#4A154B',
  discord: '#5865F2',
  'google-meet': '#00897B',
  browserscan: '#6B7280',
};

export const AgentIcon = ({ className }: { className?: string }) => (
  <img src="/alpha.svg" alt="" className={className} draggable={false} />
);

export const ProviderIcon = ({
  provider,
  className,
}: {
  provider: AccountProvider;
  className?: string;
}) => {
  const color = PROVIDER_COLOR[provider];
  const style = { color };
  switch (provider) {
    case 'whatsapp':
      return <SiWhatsapp className={className} style={style} />;
    case 'telegram':
      return <SiTelegram className={className} style={style} />;
    case 'linkedin':
      return <FaLinkedin className={className} style={style} />;
    case 'gmail':
      return <SiGmail className={className} style={style} />;
    case 'slack':
      return <SiSlack className={className} style={style} />;
    case 'discord':
      return <SiDiscord className={className} style={style} />;
    case 'google-meet':
      return <SiGooglemeet className={className} style={style} />;
    case 'browserscan':
      return <TbRobot className={className} style={style} />;
    default:
      return null;
  }
};
