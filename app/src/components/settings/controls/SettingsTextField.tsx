import { forwardRef } from 'react';

import Input, { type InputProps } from '../../ui/Input';

export interface SettingsTextFieldProps extends InputProps {
  mono?: boolean;
}

const SettingsTextField = forwardRef<HTMLInputElement, SettingsTextFieldProps>(
  ({ mono, className, ...rest }, ref) => {
    const monoClass = mono ? 'font-mono' : '';
    const merged = [monoClass, className ?? ''].filter(Boolean).join(' ');
    return <Input ref={ref} className={merged || undefined} {...rest} />;
  }
);
SettingsTextField.displayName = 'SettingsTextField';

export default SettingsTextField;
