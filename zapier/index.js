const authentication = require("./authentication");
const { addApiKey } = require("./middleware");
const validateRecord = require("./creates/validate_record");
const contract = require("./triggers/contract");
const newQuarantine = require("./triggers/new_quarantine");

const App = {
  version: require("./package.json").version,
  platformVersion: require("zapier-platform-core").version,

  authentication,
  beforeRequest: [addApiKey],
  // Blank fields are the violation. Do not let Zapier strip them before perform.
  flags: { cleanInputData: false },

  triggers: {
    [contract.key]: contract,
    [newQuarantine.key]: newQuarantine,
  },
  creates: {
    [validateRecord.key]: validateRecord,
  },
};

module.exports = App;
