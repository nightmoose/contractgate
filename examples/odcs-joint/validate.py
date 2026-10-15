import json,sys,yaml,jsonschema
from csvpath.odcs import OdcsContractLoader
c=yaml.safe_load(open("orders.odcs.yaml"))
# 1) CsvPath's own loader validation (bundled schema, no shim)
OdcsContractLoader.validate(contract=c); print("CsvPath OdcsContractLoader.validate: PASS")
OdcsContractLoader.from_path(path="orders.odcs.yaml"); print("CsvPath OdcsContractLoader.from_path: PASS")
# 2) official schema from bitol-io
s=json.load(open("official-odcs-json-schema-v3.1.0.json"))
errs=list(jsonschema.validators.validator_for(s)(s).iter_errors(c))
print("Official ODCS v3.1.0 schema:", "PASS" if not errs else f"FAIL {[e.message for e in errs]}")
