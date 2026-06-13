import EmbeddingsPanel from '../../../components/settings/panels/EmbeddingsPanel';
import CustomWizardConfigPage from './CustomWizardConfigPage';

const CustomEmbeddingsPage = () => (
  <CustomWizardConfigPage stepKey="embeddings" configureContent={<EmbeddingsPanel embedded />} />
);

export default CustomEmbeddingsPage;
