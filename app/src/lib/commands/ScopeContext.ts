import { createContext } from 'react';

export const ScopeContext = createContext<symbol>(Symbol('no-scope'));
