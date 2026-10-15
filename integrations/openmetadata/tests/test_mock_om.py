"""End-to-end against an in-process mock OpenMetadata + mock DCG HTTP server."""
import json
import threading
import unittest
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

from dcg_openmetadata.dcg import DcgClient
from dcg_openmetadata.om import OmClient
from dcg_openmetadata.publisher import Config, Mapping, publish
from dcg_openmetadata.sync import om_to_dcg

TABLE = 'svc.db.sch."orders"'
CID = "7d0c6e0e-0000-4000-8000-000000000001"


class State:
    def __init__(self):
        self.definitions, self.cases, self.results, self.imports, self.auth = {}, {}, [], [], []


def make_handler(st):
    class H(BaseHTTPRequestHandler):
        def log_message(self, *a):
            pass

        def _send(self, code, body, ctype="application/json"):
            data = body if isinstance(body, bytes) else json.dumps(body).encode()
            self.send_response(code)
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def _body(self):
            n = int(self.headers.get("Content-Length") or 0)
            return json.loads(self.rfile.read(n) or b"{}")

        def do_GET(self):
            p = urllib.parse.urlparse(self.path)
            path = urllib.parse.unquote(p.path)
            st.auth.append(self.headers.get("Authorization") or self.headers.get("x-api-key"))
            if path == f"/api/v1/tables/name/{TABLE}":
                return self._send(200, {"fullyQualifiedName": TABLE})
            if path.startswith("/api/v1/tables/name/"):
                return self._send(404, {"message": "not found"})
            if path.startswith("/api/v1/dataQuality/testDefinitions/name/"):
                n = path.rsplit("/", 1)[1]
                return self._send(200, st.definitions[n]) if n in st.definitions else self._send(404, {})
            if path.startswith("/api/v1/dataQuality/testCases/name/"):
                n = path[len("/api/v1/dataQuality/testCases/name/"):]
                return self._send(200, st.cases[n]) if n in st.cases else self._send(404, {})
            if path == "/api/v1/dataContracts/om-1/odcs/yaml":
                return self._send(200, b"apiVersion: v3.1.0\nkind: DataContract\n", "text/yaml")
            if path == f"/contracts/{CID}":
                return self._send(200, {"id": CID, "name": "orders", "latest_stable_version": "1.0.0"})
            if path == f"/contracts/{CID}/report":
                return self._send(200, {"contract_name": "orders",
                                        "totals": {"total": 10, "passed": 7, "quarantined": 3},
                                        "by_version": [{"contract_version": "1.0.0", "total": 10}],
                                        "top_violations": []})
            self._send(404, {"path": path})

        def do_POST(self):
            path = urllib.parse.unquote(urllib.parse.urlparse(self.path).path)
            b = self._body()
            if path == "/api/v1/dataQuality/testDefinitions":
                st.definitions[b["name"]] = b
                return self._send(201, b)
            if path == "/api/v1/dataQuality/testCases":
                link = b["entityLink"]
                assert link == f"<#E::table::{TABLE}>", link
                fqn = f"{TABLE}.{b['name']}"
                st.cases[fqn] = dict(b, fullyQualifiedName=fqn)
                return self._send(201, st.cases[fqn])
            pre = "/api/v1/dataQuality/testCases/testCaseResults/"
            if path.startswith(pre):
                fqn = path[len(pre):]
                if fqn not in st.cases:
                    return self._send(404, {})
                st.results.append((fqn, b))
                return self._send(200, b)
            if path == "/contracts/import":
                st.imports.append(b)
                return self._send(201, {"contract_id": b.get("target_contract_id") or "new"})
            self._send(404, {"path": path})

    return H


class MockOmTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.st = State()
        cls.srv = ThreadingHTTPServer(("127.0.0.1", 0), make_handler(cls.st))
        threading.Thread(target=cls.srv.serve_forever, daemon=True).start()
        cls.url = f"http://127.0.0.1:{cls.srv.server_port}"

    @classmethod
    def tearDownClass(cls):
        cls.srv.shutdown()

    def test_publish_creates_definition_case_and_results(self):
        om = OmClient(self.url, "jwt-123")
        dcg = DcgClient(self.url, "cg_test_key")
        cfg = Config(mappings=[Mapping(CID, table_fqn=TABLE, test_case_name="dcg_orders",
                                       max_violation_rate=0.1)])
        rows = publish(cfg, dcg, om)
        rows2 = publish(cfg, dcg, om)  # idempotent: no duplicate definition/case
        self.assertEqual(rows[0]["status"], "Failed")
        self.assertEqual(rows2[0]["status"], "Failed")
        self.assertEqual(list(self.st.definitions), ["dcgIngestContract"])
        self.assertEqual(len(self.st.cases), 1)
        self.assertEqual(len(self.st.results), 2)
        fqn, res = self.st.results[0]
        self.assertEqual(fqn, f"{TABLE}.dcg_orders")
        vals = {v["name"]: v["value"] for v in res["testResultValue"]}
        self.assertEqual(vals["quarantined"], "3")
        self.assertEqual(vals["violation_rate"], "0.300000")
        self.assertEqual(vals["contract_version"], "1.0.0")
        self.assertIn("Bearer jwt-123", self.st.auth)
        self.assertIn("cg_test_key", self.st.auth)

    def test_missing_table_reports_error(self):
        om = OmClient(self.url, "jwt")
        cfg = Config(mappings=[Mapping(CID, table_fqn="no.such.table.x")])
        rows = publish(cfg, DcgClient(self.url), om)
        self.assertEqual(rows[0]["status"], "error")
        self.assertIn("not found", rows[0]["error"])

    def test_sync_passes_target(self):
        om = OmClient(self.url, "jwt")
        out = om_to_dcg(om, DcgClient(self.url, "k"), "om-1", target_contract_id=CID)
        self.assertEqual(out["contract_id"], CID)
        self.assertTrue(self.st.imports[-1]["odcs_yaml"].startswith("apiVersion: v3.1.0"))


if __name__ == "__main__":
    unittest.main()
