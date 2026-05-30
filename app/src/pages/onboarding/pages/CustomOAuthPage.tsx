import ComposioPanel from '../../../components/settings/panels/ComposioPanel';
import CustomWizardConfigPage from './CustomWizardConfigPage';

const CustomOAuthPage = () => (
  <CustomWizardConfigPage
    stepKey="oauth"
    configureContent={<ComposioPanel embedded managedAuthEnabled={false} />}
  />
);

export default CustomOAuthPage;
