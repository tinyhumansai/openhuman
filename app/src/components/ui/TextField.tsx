import { forwardRef } from 'react';

import { cn } from '../../lib/cn';
import Input, { type InputProps } from './Input';

export interface TextFieldProps extends InputProps {
  mono?: boolean;
}

/** `Input` with an optional monospace treatment — for tokens, ids and paths. */
const TextField = forwardRef<HTMLInputElement, TextFieldProps>(
  ({ mono, className, ...rest }, ref) => (
    <Input ref={ref} className={cn(mono && 'font-mono', className) || undefined} {...rest} />
  )
);
TextField.displayName = 'TextField';

export default TextField;
