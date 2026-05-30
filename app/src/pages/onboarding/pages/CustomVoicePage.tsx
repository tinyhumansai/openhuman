import VoicePanel from '../../../components/settings/panels/VoicePanel';
import CustomWizardConfigPage from './CustomWizardConfigPage';

const CustomVoicePage = () => (
  <CustomWizardConfigPage stepKey="voice" configureContent={<VoicePanel embedded />} />
);

export default CustomVoicePage;
