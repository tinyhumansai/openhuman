import SearchPanel from '../../../components/settings/panels/SearchPanel';
import CustomWizardConfigPage from './CustomWizardConfigPage';

const CustomSearchPage = () => (
  <CustomWizardConfigPage stepKey="search" configureContent={<SearchPanel embedded />} />
);

export default CustomSearchPage;
