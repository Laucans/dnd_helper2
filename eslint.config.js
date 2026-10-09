// @ts-check
import js from '@eslint/js';
import tseslint from 'typescript-eslint';

export default tseslint.config(
  { ignores: ['target/**', 'node_modules/**', '**/pkg/**'] },
  js.configs.recommended,
  {
    files: ['packages/**/*.ts'],
    extends: [...tseslint.configs.recommendedTypeChecked],
    languageOptions: {
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
    rules: {
      // The shell writes nothing by default: diagnostics go through an injected hook (rule 53).
      'no-console': 'error',
      // A shell never reaches into a Capability, a DataCapability, WASM glue or a Micro-UI (rule 4).
      'no-restricted-imports': [
        'error',
        {
          patterns: [
            { group: ['**/crates/**', '**/apps/**', '**/pkg/**', '*.wasm', '**/*.wasm'], message: 'The shell imports no Capability, DataCapability, Micro-UI or WASM glue.' },
            { regex: '^@dnd-helper/(?!micro-ui-shell$)', message: 'The shell imports no other workspace package.' },
          ],
        },
      ],
      // No environment, no secret (rule 7); no storage (rule 53).
      'no-restricted-properties': [
        'error',
        { object: 'process', property: 'env', message: 'The shell reads no environment variable.' },
      ],
      'no-restricted-globals': [
        'error',
        { name: 'localStorage', message: 'The shell stores nothing.' },
        { name: 'sessionStorage', message: 'The shell stores nothing.' },
        { name: 'indexedDB', message: 'The shell stores nothing.' },
      ],
    },
  },
  {
    // The tests read the contract schemas from the repository, nothing else.
    files: ['packages/**/test/**/*.ts'],
    rules: {
      '@typescript-eslint/no-explicit-any': 'off',
      '@typescript-eslint/no-unsafe-assignment': 'off',
      '@typescript-eslint/no-unsafe-member-access': 'off',
      '@typescript-eslint/no-non-null-assertion': 'off',
    },
  },
);
