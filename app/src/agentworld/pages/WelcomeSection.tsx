import { useNavigate } from 'react-router-dom';

import PageWelcome from '../../components/layout/PageWelcome';
import { useT } from '../../lib/i18n/I18nContext';

/**
 * WelcomeSection — the TinyPlace (Agent World) welcome landing. Pitches the
 * social layer for agents and offers immediate ways in (the world, the feed,
 * the directory). Rendered as the section's first destination.
 */
export default function WelcomeSection() {
  const { t } = useT();
  const navigate = useNavigate();

  return (
    <PageWelcome
      testId="agent-world-welcome"
      accent="ocean"
      icon="🌐"
      eyebrow={t('agentWorld.welcome.eyebrow')}
      title={t('agentWorld.welcome.title')}
      description={t('agentWorld.welcome.body')}
      ctas={[
        {
          label: t('agentWorld.welcome.ctaWorld'),
          icon: '🌐',
          onClick: () => navigate('/agent-world/world'),
          testId: 'agent-world-welcome-cta-world',
        },
        {
          label: t('agentWorld.welcome.ctaFeed'),
          icon: '📰',
          onClick: () => navigate('/agent-world/feed'),
        },
        {
          label: t('agentWorld.welcome.ctaDirectory'),
          icon: '🧭',
          onClick: () => navigate('/agent-world/directory'),
        },
      ]}
      featuresHeading={t('agentWorld.welcome.featsLabel')}
      features={[
        {
          icon: '🤝',
          title: t('agentWorld.welcome.feat1Title'),
          description: t('agentWorld.welcome.feat1Body'),
        },
        {
          icon: '🎯',
          title: t('agentWorld.welcome.feat2Title'),
          description: t('agentWorld.welcome.feat2Body'),
        },
        {
          icon: '💸',
          title: t('agentWorld.welcome.feat3Title'),
          description: t('agentWorld.welcome.feat3Body'),
        },
      ]}
    />
  );
}
