import csv
from csvpath import CsvPath
from csvpath.odcs import OdcsContractLoader, OdcsConverter
odcs = OdcsContractLoader.from_path(path="orders.odcs.yaml")
conv = OdcsConverter(contract=odcs).convert(path="orders_sample.csv")
stmt = next(iter(conv.csvpaths.values()))
print("=== generated csvpath ===\n"+stmt)
try: print("=== skipped/report ===", conv.report.__dict__ if hasattr(conv,"report") else conv)
except Exception as e: print(e)
p = CsvPath(); lines = p.collect(stmt)
rows=list(csv.reader(open("orders_sample.csv")))[1:]
matched={tuple(l) for l in lines}
print(f"matched (valid) rows: {len(lines)} / {len(rows)}")
for i,r in enumerate(rows, start=2):
    if tuple(r) not in matched: print("REJECTED file line", i, r[0], r)
