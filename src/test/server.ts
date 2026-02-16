/**
 * MSW server instance for use in Vitest tests.
 */
import { setupServer } from 'msw/node';

import { handlers } from './handlers';

export const server = setupServer(...handlers);
