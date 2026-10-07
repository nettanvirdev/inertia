import js from "@eslint/js";
import react from "eslint-plugin-react";
import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";

export default [
  { ignores: ["dist/**", "dist-installer/**", "src-tauri/**", "installer/**", "uninstaller/**"] },
  js.configs.recommended,
  {
    files: ["src/**/*.{js,jsx}"],
    languageOptions: {
      ecmaVersion: "latest",
      sourceType: "module",
      parserOptions: { ecmaFeatures: { jsx: true } },
      globals: { ...globals.browser },
    },
    plugins: { react, "react-hooks": reactHooks },
    rules: {
      // Count `<Button />` as a use of Button; the core rule cannot see JSX.
      "react/jsx-uses-vars": "error",
      "react/jsx-uses-react": "error",
      // `const { omitted, ...rest } = x` is how a field is dropped from a copy.
      "no-unused-vars": ["error", { ignoreRestSiblings: true }],
      ...reactHooks.configs.recommended.rules,
    },
  },
  {
    files: ["src/**/*.test.{js,jsx}", "*.config.{js,ts}", "scripts/**"],
    languageOptions: { globals: { ...globals.node } },
  },
];
