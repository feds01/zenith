const plugin = require("tailwindcss/plugin");

module.exports = {
  content: ["./src/**/*.{ts,tsx}"],
  plugins: [plugin(({ addVariant }) => addVariant("hocus", ["&:hover", "&:focus"]))],
};
