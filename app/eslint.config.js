// ESLint flat config for ESLint 9+
// This config is compatible with Prettier and won't conflict with formatting rules

import js from '@eslint/js';
import tseslint from '@typescript-eslint/eslint-plugin';
import tsparser from '@typescript-eslint/parser';
import reactPlugin from 'eslint-plugin-react';
import reactHooksPlugin from 'eslint-plugin-react-hooks';
import importPlugin from 'eslint-plugin-import';
import prettierConfig from 'eslint-config-prettier';
import { fileURLToPath } from 'url';
import { dirname } from 'path';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

export default [
  // Base recommended rules
  js.configs.recommended,

  // Ignore patterns
  {
    ignores: [
      'node_modules/**',
      'target/**',
      '**/target/**',
      'dist/**',
      'dist-web/**',
      'coverage/**',
      'app/**',
      'src-tauri/**',
      'rust-core/**',
      'skills/**',
      'references/**',
      'scripts/**',
      '*.config.js',
      '*.config.ts',
      'test/vitest.config.ts',
      'tsconfig.tsbuildinfo',
    ],
  },

  // Browser environment globals
  {
    files: ['**/*.js', '**/*.ts', '**/*.jsx', '**/*.tsx'],
    languageOptions: {
      globals: {
        // Browser globals
        window: 'readonly',
        localStorage: 'readonly',
        sessionStorage: 'readonly',
        document: 'readonly',
        navigator: 'readonly',
        console: 'readonly',
        setTimeout: 'readonly',
        setInterval: 'readonly',
        clearTimeout: 'readonly',
        clearInterval: 'readonly',
        fetch: 'readonly',
        AbortSignal: 'readonly',
        self: 'readonly',
        crypto: 'readonly',
        atob: 'readonly',
        btoa: 'readonly',
        // React globals
        React: 'readonly',
        // Node.js globals (for Vite/node polyfills)
        require: 'readonly',
        process: 'readonly',
        Buffer: 'readonly',
        global: 'readonly',
        __dirname: 'readonly',
        __filename: 'readonly',
        module: 'readonly',
        exports: 'readonly',
      },
    },
  },

  // TypeScript files configuration
  {
    files: ['src/**/*.ts', 'src/**/*.tsx'],
    languageOptions: {
      parser: tsparser,
      parserOptions: {
        ecmaVersion: 'latest',
        sourceType: 'module',
        ecmaFeatures: {
          jsx: true,
        },
        project: './tsconfig.json',
        tsconfigRootDir: __dirname,
      },
    },
    plugins: {
      '@typescript-eslint': tseslint,
      import: importPlugin,
    },
    rules: {
      // Disable base no-unused-vars in favor of TypeScript version
      'no-unused-vars': 'off',
      // TypeScript recommended rules (disable base JS rules that TypeScript handles)
      '@typescript-eslint/no-unused-vars': [
        'error',
        {
          argsIgnorePattern: '^_',
          varsIgnorePattern: '^_|^[A-Z_]+$', // Ignore _prefixed vars and ALL_CAPS (enum members)
          caughtErrorsIgnorePattern: '^_',
          ignoreRestSiblings: true,
        },
      ],
      '@typescript-eslint/no-explicit-any': 'warn',
      '@typescript-eslint/explicit-function-return-type': 'off',
      '@typescript-eslint/explicit-module-boundary-types': 'off',
      '@typescript-eslint/no-non-null-assertion': 'off',

      // Import/export rules
      // Note: import/order is disabled to let Prettier handle import sorting
      // ESLint still checks for other import issues
      'import/order': 'off', // Prettier plugin handles import sorting
      'import/no-unresolved': 'off', // TypeScript handles this
      'import/no-cycle': 'warn',
      'import/no-duplicates': 'error', // Prevent duplicate imports

      // General JavaScript/TypeScript rules
      'no-console': 'off', // Allow console in frontend code
      'no-debugger': 'error',
      'no-duplicate-imports': 'error',
      'no-unused-expressions': 'off', // Covered by @typescript-eslint version
      '@typescript-eslint/no-unused-expressions': 'error',

      // Code quality
      'prefer-const': 'error',
      'no-var': 'error',
      'object-shorthand': 'error',
      'prefer-arrow-callback': 'error',

      // Style: Enforce single-line statements on same line without braces when possible
      curly: ['error', 'multi', 'consistent'], // Allow single-line without braces, require braces only for multi-statement blocks
      'nonblock-statement-body-position': ['error', 'beside'], // Enforce single-line statements on same line (prevents braces on single-line)
    },
  },

  // Barrel-import enforcement, `settings/controls` half. This one IS global:
  // it has zero outstanding deep imports app-wide (confirmed via
  // `rg "from '.*settings/controls/[A-Za-z]"`), so there is nothing to
  // grandfather.
  {
    files: ['src/**/*.ts', 'src/**/*.tsx'],
    ignores: ['src/components/settings/controls/**'],
    rules: {
      'no-restricted-imports': [
        'error',
        {
          patterns: [
            {
              group: ['**/settings/controls/*', '!**/settings/controls/index'],
              message:
                "Import settings controls from the 'settings/controls' barrel instead of a deep path into the control file.",
            },
          ],
        },
      ],
    },
  },

  // Barrel-import enforcement (S10). `src/components/ui` is documented as
  // the only sanctioned import path for shared UI primitives (see its
  // `index.ts` doc comment) and `src/components/settings/controls` is the
  // same shape for settings controls — reaching past either barrel into a
  // specific primitive file is exactly the drift this rule exists to stop
  // from regrowing once a directory has been migrated onto it.
  //
  // `**/assistant-ui/ui/*` is excluded: that is a different, barrel-less
  // vendored primitive set (shadcn-style, one file per component, no
  // `index.ts`) and deep-importing it is the intended, only way to use it.
  //
  // SCOPED, NOT GLOBAL — an allowlist, not a denylist. Turning this on
  // app-wide surfaced ~340 pre-existing deep `components/ui` imports across
  // `src/components/{accounts,BootCheckGate,channels,chat,feedback,
  // InitProgressScreen,intelligence,notifications,orchestration,rewards,
  // settings,shortcuts,skills}`, several root-level `src/components/*.tsx`
  // files, `src/features/**`, and most of `src/pages/**` — well past the 48
  // the S10 audit itself flagged, and far beyond this change's scope to fix.
  // `files` below lists exactly the directories this pass actually migrated
  // (confirmed clean via `rg "from '(\.\./)+ui/[A-Za-z]"` returning nothing
  // for each), so the rule is enforced everywhere it has already been
  // cleaned up without failing lint on code this change never touched.
  // Widen `files` as each remaining directory gets its own migration pass —
  // an allowlist only ever grows, never shrinks, so the net only tightens.
  {
    files: [
      'src/components/flows/**/*.tsx',
      'src/components/flows/**/*.ts',
      'src/components/layout/**/*.tsx',
      'src/components/layout/**/*.ts',
      'src/components/dashboard/**/*.tsx',
      'src/components/dashboard/**/*.ts',
      'src/components/approvals/**/*.tsx',
      'src/components/approvals/**/*.ts',
      'src/pages/FlowsPage.tsx',
      'src/pages/FlowCanvasPage.tsx',
    ],
    rules: {
      'no-restricted-imports': [
        'error',
        {
          patterns: [
            {
              group: ['**/ui/*', '!**/assistant-ui/ui/*', '!**/ui/index'],
              message:
                "Import UI primitives from the 'components/ui' barrel (src/components/ui) instead of a deep path into the primitive file.",
            },
            // Repeated verbatim from the global block below. Flat config
            // REPLACES a rule's options rather than merging them, so a later
            // block that also sets `no-restricted-imports` would otherwise
            // silently drop whichever patterns it does not itself list. Both
            // halves therefore have to travel together in every block that
            // configures this rule.
            {
              group: ['**/settings/controls/*', '!**/settings/controls/index'],
              message:
                "Import settings controls from the 'settings/controls' barrel instead of a deep path into the control file.",
            },
          ],
        },
      ],
    },
  },

  // Frontend config is centralized in src/utils/config.ts (AGENTS.md). A direct
  // import.meta.env read elsewhere bypasses the derived values that file owns --
  // IS_DEV_LIKE exists precisely because import.meta.env.DEV is false under the
  // E2E harness (vite build --mode development) -- and cannot be stubbed once the
  // module graph has loaded, which is why the loopback OAuth test reads config
  // instead. The rule was documented but unenforced, and had already drifted.
  {
    files: ['src/**/*.ts', 'src/**/*.tsx'],
    ignores: [
      'src/utils/config.ts',
      'src/test/**',
      'src/**/__tests__/**',
      'src/**/*.test.ts',
      'src/**/*.test.tsx',
    ],
    rules: {
      'no-restricted-syntax': [
        'error',
        {
          // Pin the meta-property to `import.meta` -- `new.target` is a MetaProperty
          // too -- and match both `import.meta.env` and `import.meta['env']`.
          selector:
            'MemberExpression[object.type="MetaProperty"][object.meta.name="import"][object.property.name="meta"]:matches([computed=false][property.name="env"], [computed=true][property.value="env"])',
          message:
            'Read frontend config from src/utils/config.ts (IS_DEV, IS_DEV_LIKE, ...) instead of import.meta.env directly; add the value there if it is missing.',
        },
      ],
    },
  },

  // React files configuration
  {
    files: ['src/**/*.jsx', 'src/**/*.tsx'],
    languageOptions: {
      parser: tsparser,
      parserOptions: {
        ecmaVersion: 'latest',
        sourceType: 'module',
        ecmaFeatures: {
          jsx: true,
        },
        project: './tsconfig.json',
        tsconfigRootDir: __dirname,
      },
    },
    plugins: {
      react: reactPlugin,
      'react-hooks': reactHooksPlugin,
    },
    settings: {
      react: {
        version: 'detect',
      },
    },
    rules: {
      ...reactPlugin.configs.recommended.rules,
      ...reactHooksPlugin.configs.recommended.rules,
      'react/react-in-jsx-scope': 'off', // Not needed in React 17+
      'react/prop-types': 'off', // TypeScript handles prop validation
      'react/display-name': 'off', // Not needed with TypeScript
      'react/no-unescaped-entities': 'off', // Prettier handles this
      'react-hooks/rules-of-hooks': 'error',
      'react-hooks/exhaustive-deps': 'warn',
      'react-hooks/set-state-in-effect': 'warn', // Allow initialization in effects
      'react-hooks/refs': 'off', // Allow ref access in context providers
    },
  },

  // Vitest test files and test setup files (must come after TypeScript config to override rules)
  {
    files: [
      '**/*.test.ts',
      '**/*.test.tsx',
      '**/*.spec.ts',
      '**/*.spec.tsx',
      '**/__tests__/**/*.ts',
      '**/__tests__/**/*.tsx',
    ],
    languageOptions: {
      globals: {
        // Vitest globals
        describe: 'readonly',
        it: 'readonly',
        test: 'readonly',
        expect: 'readonly',
        beforeEach: 'readonly',
        afterEach: 'readonly',
        beforeAll: 'readonly',
        afterAll: 'readonly',
        vi: 'readonly',
        vitest: 'readonly',
      },
    },
    rules: {
      '@typescript-eslint/no-explicit-any': 'off', // Allow any in tests
      '@typescript-eslint/no-non-null-assertion': 'off', // Allow non-null assertions in tests
      'no-undef': 'off', // Vitest provides globals
    },
  },

  // Unit test files in test/ — TypeScript + JSX, parsed with main tsconfig
  {
    files: ['test/*.test.ts', 'test/*.test.tsx'],
    languageOptions: {
      parser: tsparser,
      parserOptions: {
        ecmaVersion: 'latest',
        sourceType: 'module',
        ecmaFeatures: { jsx: true },
        project: './test/tsconfig.unit.json',
        tsconfigRootDir: __dirname,
      },
      globals: {
        describe: 'readonly',
        it: 'readonly',
        test: 'readonly',
        expect: 'readonly',
        beforeEach: 'readonly',
        afterEach: 'readonly',
        beforeAll: 'readonly',
        afterAll: 'readonly',
        vi: 'readonly',
      },
    },
    plugins: {
      '@typescript-eslint': tseslint,
      react: reactPlugin,
      'react-hooks': reactHooksPlugin,
    },
    settings: { react: { version: 'detect' } },
    rules: {
      'react/react-in-jsx-scope': 'off',
      'react/prop-types': 'off',
      '@typescript-eslint/no-explicit-any': 'off',
      'no-unused-vars': 'off',
      '@typescript-eslint/no-unused-vars': [
        'error',
        { argsIgnorePattern: '^_', varsIgnorePattern: '^_', caughtErrorsIgnorePattern: '^_' },
      ],
      'no-undef': 'off',
    },
  },

  // E2E test files (WDIO + Playwright) — use tsconfig.e2e.json for parsing
  {
    files: ['test/e2e/**/*.ts', 'test/playwright/**/*.ts', 'test/wdio.conf.ts'],
    languageOptions: {
      parser: tsparser,
      parserOptions: {
        ecmaVersion: 'latest',
        sourceType: 'module',
        project: './test/tsconfig.e2e.json',
        tsconfigRootDir: __dirname,
      },
      globals: {
        browser: 'readonly',
        $: 'readonly',
        $$: 'readonly',
        describe: 'readonly',
        it: 'readonly',
        before: 'readonly',
        after: 'readonly',
        beforeEach: 'readonly',
        afterEach: 'readonly',
        expect: 'readonly',
      },
    },
    plugins: {
      '@typescript-eslint': tseslint,
    },
    rules: {
      'no-unused-vars': 'off',
      '@typescript-eslint/no-unused-vars': [
        'error',
        { argsIgnorePattern: '^_', varsIgnorePattern: '^_', caughtErrorsIgnorePattern: '^_' },
      ],
      '@typescript-eslint/no-explicit-any': 'off',
      'no-undef': 'off',
    },
  },

  // Playwright test helpers/specs are intentionally more permissive:
  // empty catch blocks are used for best-effort browser-lane fallbacks and
  // many helpers keep optional args/imports for parity with the WDIO suite.
  {
    files: ['test/playwright/**/*.ts'],
    rules: {
      'no-empty': 'off',
      'no-unused-vars': 'off',
      '@typescript-eslint/no-unused-vars': 'off',
    },
  },

  // JavaScript files configuration
  {
    files: ['**/*.js', '**/*.jsx'],
    languageOptions: { ecmaVersion: 'latest', sourceType: 'module' },
    rules: {
      'no-unused-vars': ['error', { argsIgnorePattern: '^_', varsIgnorePattern: '^_' }],
      'no-console': 'off',
      'no-debugger': 'error',
      'prefer-const': 'error',
      'no-var': 'error',
    },
  },

  // Disable all Prettier-conflicting rules (must be last)
  prettierConfig,
];
