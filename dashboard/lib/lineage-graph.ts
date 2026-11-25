/**
 * Pure data shaping for the lineage graph view: merge the org graph
 * (GET /contracts/{id}/lineage/graph) with an impact report's affected[]
 * (GET .../impact) and lay the result out with dagre. Kept framework-free
 * so it is unit-testable without a DOM or React Flow.
 */

import dagre from "@dagrejs/dagre";
import type { ImpactAsset, LineageGraph } from "./api";

export type NodeSeverity = "breaking" | "safe" | "none" | "unaffected" | "unknown";

// The index signature satisfies React Flow's `Node<T extends
// Record<string, unknown>>` / `Edge<T extends Record<string, unknown>>`
// constraint used by the graph component.
export interface GraphNodeData {
  [key: string]: unknown;
  key: string;
  kind: string;
  name: string;
  owner: string | null;
  isFocus: boolean;
  severity: NodeSeverity;
  hops: number | null;
  path: string[];
  changes: ImpactAsset["changes"];
}

export interface GraphEdgeData {
  [key: string]: unknown;
  edgeKind: string;
  field: string | null;
}

export interface LayoutNode {
  id: string;
  position: { x: number; y: number };
  data: GraphNodeData;
}

export interface LayoutEdge {
  id: string;
  source: string;
  target: string;
  data: GraphEdgeData;
}

const NODE_WIDTH = 190;
const NODE_HEIGHT = 60;

export function indexAffected(affected: ImpactAsset[]): Map<string, ImpactAsset> {
  return new Map(affected.map((asset) => [asset.key, asset]));
}

/** `affected` is null when no version's report has been loaded yet. */
function severityFor(
  key: string,
  affectedByKey: Map<string, ImpactAsset> | null,
): Pick<GraphNodeData, "severity" | "hops" | "path" | "changes"> {
  if (!affectedByKey) {
    return { severity: "unknown", hops: null, path: [], changes: [] };
  }
  const asset = affectedByKey.get(key);
  if (!asset) {
    return { severity: "unaffected", hops: null, path: [], changes: [] };
  }
  const severity: NodeSeverity =
    asset.severity === "breaking" || asset.severity === "safe" || asset.severity === "none"
      ? asset.severity
      : "none";
  return { severity, hops: asset.hops, path: asset.path, changes: asset.changes };
}

export function buildGraphElements(
  graph: LineageGraph,
  affected: ImpactAsset[] | null,
): { nodes: LayoutNode[]; edges: LayoutEdge[] } {
  const affectedByKey = affected ? indexAffected(affected) : null;
  const focusSet = new Set(graph.focus_keys);
  const nodes: LayoutNode[] = graph.assets.map((asset) => ({
    id: asset.key,
    position: { x: 0, y: 0 },
    data: {
      key: asset.key,
      kind: asset.kind,
      name: asset.name,
      owner: asset.owner,
      isFocus: focusSet.has(asset.key),
      ...severityFor(asset.key, affectedByKey),
    },
  }));
  const edges: LayoutEdge[] = graph.edges.map((edge, index) => ({
    id: `${edge.from}->${edge.to}:${edge.field ?? ""}:${index}`,
    source: edge.from,
    target: edge.to,
    data: { edgeKind: edge.edge, field: edge.field },
  }));
  return { nodes, edges };
}

/** Assigns dagre-computed positions in place of each node's (0, 0). */
export function layoutGraph(
  nodes: LayoutNode[],
  edges: LayoutEdge[],
  direction: "LR" | "TB" = "LR",
): LayoutNode[] {
  const g = new dagre.graphlib.Graph();
  g.setGraph({ rankdir: direction, nodesep: 36, ranksep: 90 });
  g.setDefaultEdgeLabel(() => ({}));
  for (const node of nodes) {
    g.setNode(node.id, { width: NODE_WIDTH, height: NODE_HEIGHT });
  }
  for (const edge of edges) {
    if (g.hasNode(edge.source) && g.hasNode(edge.target)) {
      g.setEdge(edge.source, edge.target);
    }
  }
  dagre.layout(g);
  return nodes.map((node) => {
    const positioned = g.node(node.id);
    if (!positioned) return node;
    return {
      ...node,
      position: { x: positioned.x - NODE_WIDTH / 2, y: positioned.y - NODE_HEIGHT / 2 },
    };
  });
}

/**
 * Restricts the graph to whatever is weakly connected (either direction)
 * to the focus keys — the contract-focused view. An org with several
 * unrelated lineage imports should not clutter one contract's graph with
 * disconnected components.
 */
export function focusSubgraph(
  nodes: LayoutNode[],
  edges: LayoutEdge[],
  focusKeys: string[],
): { nodes: LayoutNode[]; edges: LayoutEdge[] } {
  if (focusKeys.length === 0) {
    return { nodes, edges };
  }
  const adjacency = new Map<string, string[]>();
  for (const edge of edges) {
    adjacency.set(edge.source, [...(adjacency.get(edge.source) ?? []), edge.target]);
    adjacency.set(edge.target, [...(adjacency.get(edge.target) ?? []), edge.source]);
  }
  const reachable = new Set<string>();
  const queue = [...focusKeys];
  while (queue.length > 0) {
    const key = queue.shift() as string;
    if (reachable.has(key)) continue;
    reachable.add(key);
    for (const next of adjacency.get(key) ?? []) queue.push(next);
  }
  return {
    nodes: nodes.filter((node) => reachable.has(node.id)),
    edges: edges.filter((edge) => reachable.has(edge.source) && reachable.has(edge.target)),
  };
}

export function severityColor(severity: NodeSeverity): string {
  switch (severity) {
    case "breaking":
      return "#f43f5e";
    case "safe":
      return "#10b981";
    case "none":
      return "#f59e0b";
    case "unaffected":
      return "#334155";
    default:
      return "#475569";
  }
}
