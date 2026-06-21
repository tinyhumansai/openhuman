import { FaLinkedin, FaWeixin } from 'react-icons/fa';
import {
  SiDiscord,
  SiGmail,
  SiGooglemeet,
  SiInstagram,
  SiSlack,
  SiTelegram,
  SiWhatsapp,
  SiX,
  SiZoom,
} from 'react-icons/si';
import { TbMail, TbRobot } from 'react-icons/tb';

import type { AccountProvider } from '../../types/accounts';

/**
 * Brand colors for the provider icons — matches each service's own
 * marketing identity. Kept in one place so they stay consistent wherever
 * the icon is reused (sidebar rail, add-account modal, etc.).
 */
const PROVIDER_COLOR: Record<AccountProvider, string> = {
  whatsapp: '#25D366',
  wechat: '#07C160',
  telegram: '#229ED9',
  linkedin: '#0A66C2',
  slack: '#4A154B',
  discord: '#5865F2',
  gmail: '#EA4335',
  outlook: '#0F6CBD',
  instagram: '#E4405F',
  twitter: '#000000',
  'google-meet': '#00897B',
  zoom: '#2D8CFF',
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
    case 'wechat':
      return <FaWeixin className={className} style={style} />;
    case 'telegram':
      return <SiTelegram className={className} style={style} />;
    case 'linkedin':
      return <FaLinkedin className={className} style={style} />;
    case 'slack':
      return <SiSlack className={className} style={style} />;
    case 'discord':
      return <SiDiscord className={className} style={style} />;
    case 'gmail':
      return <SiGmail className={className} style={style} />;
    case 'outlook':
      return <TbMail className={className} style={style} />;
    case 'instagram':
      return <SiInstagram className={className} style={style} />;
    case 'twitter':
      return <SiX className={className} style={style} />;
    case 'google-meet':
      return <SiGooglemeet className={className} style={style} />;
    case 'zoom':
      return <SiZoom className={className} style={style} />;
    case 'browserscan':
      return <TbRobot className={className} style={style} />;
    default:
      return null;
  }
};
