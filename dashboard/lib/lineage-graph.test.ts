import assert from "node:assert/strict";
import test from "node:test";
import { buildGraphElements, focusSubgraph, layoutGraph, severityColor } from "./lineage-graph";
import type { ImpactAsset, LineageGraph } from "./api";

const graph: LineageGraph = {
  asset_count: 3,
  edge_count: 2,
  focus_keys: ["contract:orders"],
  assets: [
    { kind: "contract", key: "contract:orders", name: "orders", owner: null, contract_id: "orders-id" },
    { kind: "model", key: "model:stg_orders", name: "stg_orders", owner: "eng@example.com", contract_id: null },
    { kind: "dashboard", key: "dashboard:revenue", name: "Revenue", owner: "bi@example.com", contract_id: null },
  ],
  edges: [
    { from: "contract:orders", to: "model:stg_orders", edge: "feeds", field: null },
    { from: "model:stg_orders", to: "dashboard:revenue", edge: "displays", field: "amount" },
  ],
};

const affected: ImpactAsset[] = [
  {
    key: "model:stg_orders",
    kind: "model",
    name: "stg_orders",
    owner: "eng@example.com",
    hops: 1,
    path: ["orders", "stg_orders"],
    explanation: "stg_orders is 1 hop downstream. Breaking: amount was removed.",
    severity: "breaking",
    changes: [
      { field: "amount", kind: "field_removed", severity: "breaking", detail: "removed", explanation: "Field `amount` was removed." },
    ],
  },
];

test("buildGraphElements marks the contract's own node as focus", () => {
  const { nodes } = buildGraphElements(graph, null);
  const contractNode = nodes.find((n) => n.id === "contract:orders");
  assert.ok(contractNode);
  assert.equal(contractNode?.data.isFocus, true);
  const modelNode = nodes.find((n) => n.id === "model:stg_orders");
  assert.equal(modelNode?.data.isFocus, false);
});

test("buildGraphElements leaves nodes unknown with no report loaded", () => {
  const { nodes } = buildGraphElements(graph, null);
  for (const node of nodes) {
    assert.equal(node.data.severity, "unknown");
  }
});

test("buildGraphElements overlays severity and reaching changes from the impact report", () => {
  const { nodes } = buildGraphElements(graph, affected);
  const stgOrders = nodes.find((n) => n.id === "model:stg_orders");
  assert.equal(stgOrders?.data.severity, "breaking");
  assert.equal(stgOrders?.data.hops, 1);
  assert.equal(stgOrders?.data.changes.length, 1);

  const revenue = nodes.find((n) => n.id === "dashboard:revenue");
  assert.equal(revenue?.data.severity, "unaffected");

  const contract = nodes.find((n) => n.id === "contract:orders");
  assert.equal(contract?.data.severity, "unaffected");
});

test("buildGraphElements carries the edge field scope through for hover labels", () => {
  const { edges } = buildGraphElements(graph, affected);
  const scoped = edges.find((e) => e.source === "model:stg_orders" && e.target === "dashboard:revenue");
  assert.equal(scoped?.data.field, "amount");
  const unscoped = edges.find((e) => e.source === "contract:orders" && e.target === "model:stg_orders");
  assert.equal(unscoped?.data.field, null);
});

test("layoutGraph assigns distinct, finite positions to every node", () => {
  const { nodes, edges } = buildGraphElements(graph, affected);
  const laidOut = layoutGraph(nodes, edges);
  assert.equal(laidOut.length, nodes.length);
  for (const node of laidOut) {
    assert.ok(Number.isFinite(node.position.x));
    assert.ok(Number.isFinite(node.position.y));
  }
  const xs = new Set(laidOut.map((n) => n.position.x));
  assert.ok(xs.size > 1, "nodes along the chain should not all share an x position");
});

test("layoutGraph tolerates an edge referencing a node outside the asset list", () => {
  const { nodes, edges } = buildGraphElements(graph, affected);
  edges.push({ id: "stray", source: "model:stg_orders", target: "job:unknown_sync", data: { edgeKind: "feeds", field: null } });
  const laidOut = layoutGraph(nodes, edges);
  assert.equal(laidOut.length, nodes.length);
});

test("focusSubgraph keeps only the component connected to the focus keys", () => {
  const { nodes, edges } = buildGraphElements(
    {
      ...graph,
      assets: [...graph.assets, { kind: "model", key: "model:unrelated", name: "unrelated", owner: null, contract_id: null }],
    },
    null,
  );
  const { nodes: scoped, edges: scopedEdges } = focusSubgraph(nodes, edges, graph.focus_keys);
  assert.deepEqual(
    scoped.map((n) => n.id).sort(),
    ["contract:orders", "dashboard:revenue", "model:stg_orders"],
  );
  assert.equal(scopedEdges.length, 2);
});

test("focusSubgraph is a no-op when there are no focus keys", () => {
  const { nodes, edges } = buildGraphElements(graph, null);
  const result = focusSubgraph(nodes, edges, []);
  assert.equal(result.nodes.length, nodes.length);
  assert.equal(result.edges.length, edges.length);
});

test("severityColor returns a distinct color per severity", () => {
  const colors = new Set(
    ["breaking", "safe", "none", "unaffected", "unknown"].map((s) => severityColor(s as never)),
  );
  assert.equal(colors.size, 5);
});
