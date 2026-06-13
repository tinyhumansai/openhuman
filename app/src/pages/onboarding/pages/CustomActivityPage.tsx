import AgentActivityPanel from '../../../components/settings/panels/AgentActivityPanel';
import CustomWizardConfigPage from './CustomWizardConfigPage';

const CustomActivityPage = () => (
  <CustomWizardConfigPage stepKey="activity" configureContent={<AgentActivityPanel />} />
);

export default CustomActivityPage;
