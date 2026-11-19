import globals from "globals";
export default [
  {
    files: ["ui/**/*.js"],
    languageOptions: {
      ecmaVersion: 2023,
      sourceType: "module",
      globals: globals.browser,
    },
    rules: {
      "no-undef": "error",
      "no-unused-vars": ["error", { args: "none" }],
      "no-unreachable": "error",
    },
  },
];
