import json
import os
import tempfile
import unittest
from datetime import datetime, timezone

from dcg_openmetadata import publisher as P
from dcg_openmetadata.sync import diff_extensions, extension_keys

REPORT = {
    "contract_id": "c1", "contract_name": "orders",
    "totals": {"total": 200, "passed": 190, "quarantined": 10, "pass_rate": 0.95},
    "by_version": [{"contract_version": "1.0.0", "total": 20, "passed": 20, "quarantined": 0},
                   {"contract_version": "1.1.0", "total": 180, "passed": 170, "quarantined": 10}],
    "top_violations": [{"field": "amount", "kind": "min", "count": 10}],
}


class BuildResult(unittest.TestCase):
    def values(self, r):
        return {v["name"]: v["value"] for v in r["testResultValue"]}

    def test_failed_over_threshold(self):
        r = P.build_result(REPORT, "1.1.0", 0.01, now_ms=1)
        self.assertEqual(r["testCaseStatus"], "Failed")
        v = self.values(r)
        self.assertEqual(v["quarantined"], "10")
        self.assertEqual(v["violation_rate"], "0.050000")
        self.assertEqual(v["contract_version"], "1.1.0")
        self.assertEqual(r["failedRows"], 10)
        self.assertIn("amount:min x10", r["result"])

    def test_success_under_threshold(self):
        self.assertEqual(P.build_result(REPORT, "1.1.0", 0.1)["testCaseStatus"], "Success")

    def test_zero_events(self):
        r = P.build_result({"totals": {"total": 0}}, "1", 0.0)
        self.assertEqual(r["testCaseStatus"], "Success")
        self.assertEqual(self.values(r)["violation_rate"], "0.000000")


class Helpers(unittest.TestCase):
    def test_pick_version(self):
        self.assertEqual(P.pick_version(REPORT, {}), "1.1.0")
        self.assertEqual(P.pick_version({"by_version": []}, {"latest_stable_version": "2"}), "2")

    def test_fqn_from_yaml_and_description(self):
        y = 'version: "1"\nx-openmetadata-table: "svc.db.sch.tbl"\nname: a\n'
        self.assertEqual(P.fqn_from_metadata({}, y), "svc.db.sch.tbl")
        self.assertEqual(P.fqn_from_metadata({"description": "x openmetadata-table: a.b.c.d"}, None),
                         "a.b.c.d")
        self.assertIsNone(P.fqn_from_metadata({"description": "nothing"}, "name: x"))

    def test_default_test_name(self):
        self.assertEqual(P.default_test_name("user events-v2"), "dcg_user_events_v2")

    def test_load_config(self):
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
            json.dump({"window_minutes": 5, "defaults": {"max_violation_rate": 0.2},
                       "contracts": [{"contract_id": "a"}, {"contract_id": "b",
                                                            "max_violation_rate": 0.5}]}, f)
        try:
            c = P.load_config(f.name)
        finally:
            os.unlink(f.name)
        self.assertEqual(c.window_minutes, 5)
        self.assertEqual([m.max_violation_rate for m in c.mappings], [0.2, 0.5])

    def test_extension_diff(self):
        before = "x-contractgate-ontology:\n  a: 1\nschema:\n  - customProperties:\n" \
                 "      - property: x-contractgate-pattern\n"
        after = "schema:\n  - customProperties:\n      - property: x-contractgate-pattern\n"
        self.assertEqual(extension_keys(before)["top_level"], {"x-contractgate-ontology"})
        d = diff_extensions(before, after)
        self.assertEqual(d["top_level"]["lost"], ["x-contractgate-ontology"])
        self.assertEqual(d["custom_properties"]["kept"], ["x-contractgate-pattern"])


class FakeDcg:
    def __init__(self, report=REPORT):
        self._report = report
        self.since = None

    def get_contract(self, cid):
        return {"id": cid, "name": "orders", "latest_stable_version": "1.1.0",
                "description": "openmetadata-table: svc.db.sch.orders"}

    def get_version(self, cid, v):
        return {"yaml_content": "name: orders\n"}

    def report(self, cid, since=None, until=None):
        self.since = since
        return self._report


class PublishWithFakes(unittest.TestCase):
    def test_dry_run_and_window(self):
        cfg = P.Config(mappings=[P.Mapping("c1")], window_minutes=60)
        dcg = FakeDcg()
        rows = P.publish(cfg, dcg, om=None, dry_run=True,
                         now=datetime(2026, 1, 1, 12, tzinfo=timezone.utc))
        self.assertEqual(dcg.since, "2026-01-01T11:00:00Z")
        self.assertEqual(rows[0]["test_case"], "svc.db.sch.orders.dcg_orders")
        self.assertEqual(rows[0]["status"], "Failed")

    def test_skip_empty(self):
        cfg = P.Config(mappings=[P.Mapping("c1", table_fqn="a.b.c.d")])
        rows = P.publish(cfg, FakeDcg({"totals": {"total": 0}}), om=None, dry_run=True)
        self.assertTrue(rows[0]["status"].startswith("skipped"))


if __name__ == "__main__":
    unittest.main()


try:
    import yaml  # noqa: F401
    HAVE_YAML = True
except ImportError:
    HAVE_YAML = False


@unittest.skipUnless(HAVE_YAML, "PyYAML not installed")
class OmCompatShim(unittest.TestCase):
    def test_description_and_custom_properties(self):
        import yaml
        from dcg_openmetadata.sync import dcg_odcs_for_om
        src = ("apiVersion: v3.1.0\nkind: DataContract\ndescription: 'hello'\nschema:\n"
               "- name: t\n  properties:\n  - name: a\n    customProperties:\n"
               "    - property: x-cg-min\n      value: 0\nx-contractgate-ontology: {a: 1}\n")
        out = yaml.safe_load(dcg_odcs_for_om(src))
        self.assertEqual(out["description"], {"purpose": "hello"})
        self.assertNotIn("customProperties", out["schema"][0]["properties"][0])
        self.assertEqual(out["x-contractgate-ontology"], {"a": 1})

    def test_strict_export_nested_custom_properties_and_root_values(self):
        import json
        import yaml
        from dcg_openmetadata.sync import dcg_odcs_for_om
        src = yaml.safe_dump({
            "apiVersion": "v3.1.0", "kind": "DataContract", "id": "x", "version": "1.0.0",
            "status": "active", "description": {"purpose": "hello"},
            "schema": [{"name": "t", "logicalType": "object", "properties": [{
                "name": "a", "logicalType": "string",
                "customProperties": [{"property": "x-cg-enum", "value": ["A"]}],
                "quality": [{"type": "library", "metric": "invalidValues", "mustBe": 0,
                             "arguments": {"validValues": ["A"]},
                             "customProperties": [{"property": "x-cg-derived",
                                                   "value": "enum"}]}],
                "items": {"name": "i", "customProperties": []},
            }]}],
            "customProperties": [
                {"property": "x-contractgate-version", "value": "1.0"},
                {"property": "x-contractgate-ontology", "value": {"a": 1}},
            ],
        })
        out = yaml.safe_load(dcg_odcs_for_om(src))
        self.assertEqual(out["description"], {"purpose": "hello"})
        prop = out["schema"][0]["properties"][0]
        self.assertNotIn("customProperties", prop)
        self.assertNotIn("customProperties", prop["quality"][0])
        self.assertNotIn("customProperties", prop["items"])
        self.assertEqual(prop["quality"][0]["arguments"]["validValues"], ["A"])
        root = {c["property"]: c["value"] for c in out["customProperties"]}
        self.assertEqual(root["x-contractgate-version"], "1.0")
        self.assertEqual(json.loads(root["x-contractgate-ontology"]), {"a": 1})

