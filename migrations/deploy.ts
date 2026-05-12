// Migrations are an early feature. Currently, they're nothing more than this
// temporary script that's executed when running `anchor deploy`. In the future,
// they'll be required for account migrations.

const anchor = require("@coral-xyz/anchor");

module.exports = async function (provider) {
  // Configure client to use the provider.
  anchor.setProvider(provider);

  console.log("Migration complete!");
};
