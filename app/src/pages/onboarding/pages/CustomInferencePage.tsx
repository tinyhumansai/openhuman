import AIPanel from '../../../components/settings/panels/AIPanel';
import CustomWizardConfigPage from './CustomWizardConfigPage';

const CustomInferencePage = () => (
  <CustomWizardConfigPage
    stepKey="inference"
    backRoute="/onboarding/runtime-choice"
    configureContent={<AIPanel embedded />}
  />
);

export default CustomInferencePage;
