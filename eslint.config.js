// ESLint flat config for ESLint 9+
// This config is compatible with Prettier and won't conflict with formatting rules

const js = require('@eslint/js');
const tseslint = require('@typescript-eslint/eslint-plugin');
const tsparser = require('@typescript-eslint/parser');
const reactPlugin = require('eslint-plugin-react');
const reactHooksPlugin = require('eslint-plugin-react-hooks');
const importPlugin = require('eslint-plugin-import');
const prettierConfig = require('eslint-config-prettier');

module.exports = [
  // Base recommended rules
  js.configs.recommended,

  // Ignore patterns
  {
    ignores: [
      'node_modules/**',
      'dist/**',
      'coverage/**',
      'src-tauri/**',
      'skills/**',
      '*.config.js',
      '*.config.ts',
      'vitest.config.ts',
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
        document: 'readonly',
        navigator: 'readonly',
        console: 'readonly',
        setTimeout: 'readonly',
        setInterval: 'readonly',
        clearTimeout: 'readonly',
        clearInterval: 'readonly',
        fetch: 'readonly',
        AbortSignal: 'readonly',
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
    files: ['**/*.ts', '**/*.tsx'],
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

  // React files configuration
  {
    files: ['**/*.jsx', '**/*.tsx'],
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
      'react-hooks/rules-of-hooks': 'error',
      'react-hooks/exhaustive-deps': 'warn',
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
