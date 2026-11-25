"use client";

/**
 * Interactive lineage graph for the Downstream impact section: assets and
 * edges as nodes/edges, colored by the selected version's breaking changes.
 * Falls back to a plain list (rendered by the caller) on 402/503/error.
 */

import { useEffect, useMemo, useRef, useState } from "react";
import {
  ReactFlow,
  ReactFlowProvider,
  Background,
  Controls,
  MiniMap,
  Handle,
  Position,
  BaseEdge,
  getBezierPath,
  type Node,
  type Edge,
  type NodeProps,
  type EdgeProps,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { getContractLineageGraph, type ImpactAsset, type LineageGraph } from "@/lib/api";
import { localUpgradeMessage } from "@/lib/impact-upgrade";
import {
  buildGraphElements,
  focusSubgraph,
  layoutGraph,
  severityColor,
  type GraphEdgeData,
  type GraphNodeData,
} from "@/lib/lineage-graph";

type LineageNodeType = Node<GraphNodeData, "lineage">;
type LineageEdgeType = Edge<GraphEdgeData, "lineage">;

function severityLabel(severity: GraphNodeData["severity"]): string | null {
  switch (severity) {
    case "breaking":
      return "Breaking";
    case "safe":
      return "Safe";
    case "none":
      return "Reached";
    default:
      return null;
  }
}

function LineageNode({ data, selected }: NodeProps<LineageNodeType>) {
  const color = severityColor(data.severity);
  const label = severityLabel(data.severity);
  return (
    <div
      data-testid="lineage-node"
      className="rounded-lg px-3 py-2 text-left bg-[#0a0d12]"
      style={{
        border: `${data.isFocus ? 2 : 1}px solid ${selected ? "#e2e8f0" : color}`,
        minWidth: 170,
        maxWidth: 190,
      }}
      title={data.owner ? `${data.name} · owner: ${data.owner}` : data.name}
    >
      <Handle type="target" position={Position.Left} style={{ background: color, width: 8, height: 8 }} />
      <p className="text-[10px] uppercase tracking-wide text-slate-500">{data.kind}</p>
      <p className="text-sm text-slate-100 truncate">{data.name}</p>
      {label && (
        <p className="text-[11px]" style={{ color }}>
          {label}
          {data.hops !== null ? ` · ${data.hops} ${data.hops === 1 ? "hop" : "hops"}` : ""}
        </p>
      )}
      <Handle type="source" position={Position.Right} style={{ background: color, width: 8, height: 8 }} />
    </div>
  );
}

function LineageEdge({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  style,
  markerEnd,
  data,
}: EdgeProps<LineageEdgeType>) {
  const [path, labelX, labelY] = getBezierPath({
    sourceX,
    sourceY,
    targetX,
    targetY,
    sourcePosition,
    targetPosition,
  });
  const [hovered, setHovered] = useState(false);
  const showLabel = hovered && !!data?.field;
  return (
    <>
      <BaseEdge id={id} path={path} style={style} markerEnd={markerEnd} />
      {/* Wider, invisible hit area so hovering near (not just exactly on) the
          thin edge path reveals the field scope. */}
      <path
        d={path}
        fill="none"
        stroke="transparent"
        strokeWidth={16}
        onMouseEnter={() => setHovered(true)}
        onMouseLeave={() => setHovered(false)}
      />
      {showLabel && (
        <foreignObject x={labelX - 70} y={labelY - 11} width={140} height={22} style={{ overflow: "visible", pointerEvents: "none" }}>
          <div className="text-[10px] text-center bg-[#111827] border border-[#374151] text-amber-300 rounded px-1.5 py-0.5 truncate">
            field: {data?.field}
          </div>
        </foreignObject>
      )}
    </>
  );
}

const NODE_TYPES = { lineage: LineageNode };
const EDGE_TYPES = { lineage: LineageEdge };

interface LineageGraphViewProps {
  contractId: string;
  affected: ImpactAsset[] | null;
  /** Called once when the graph cannot be shown (not a 402), so the caller can fall back to its list view. */
  onUnavailable?: () => void;
}

type LoadState =
  | { status: "loading" }
  | { status: "ok"; graph: LineageGraph }
  | { status: "upgrade"; message: string }
  | { status: "unavailable"; message: string }
  | { status: "error"; message: string };

export function LineageGraphView({ contractId, affected, onUnavailable }: LineageGraphViewProps) {
  const [state, setState] = useState<LoadState>({ status: "loading" });
  const [scope, setScope] = useState<"contract" | "org">("contract");
  const [selected, setSelected] = useState<GraphNodeData | null>(null);
  const controllerRef = useRef<AbortController | null>(null);

  useEffect(() => {
    controllerRef.current?.abort();
    const controller = new AbortController();
    controllerRef.current = controller;
    setState({ status: "loading" });
    setSelected(null);
    void getContractLineageGraph(contractId, controller.signal).then((result) => {
      if (controller.signal.aborted) return;
      if (result.status === "ok") {
        setState({ status: "ok", graph: result.graph });
      } else if (result.status === "upgrade") {
        setState({ status: "upgrade", message: localUpgradeMessage(result.message) });
      } else {
        setState({ status: result.status, message: result.message });
        onUnavailable?.();
      }
    }).catch((err) => {
      if (controller.signal.aborted) return;
      setState({ status: "error", message: err instanceof Error ? err.message : "Lineage graph request failed" });
      onUnavailable?.();
    });
    return () => controller.abort();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [contractId]);

  const { nodes, edges } = useMemo(() => {
    if (state.status !== "ok") return { nodes: [] as LineageNodeType[], edges: [] as LineageEdgeType[] };
    const built = buildGraphElements(state.graph, affected);
    const scoped = scope === "contract" ? focusSubgraph(built.nodes, built.edges, state.graph.focus_keys) : built;
    const laidOut = layoutGraph(scoped.nodes, scoped.edges);
    const flowNodes: LineageNodeType[] = laidOut.map((node) => ({
      id: node.id,
      type: "lineage",
      position: node.position,
      data: node.data,
      selected: node.id === selected?.key,
    }));
    const flowEdges: LineageEdgeType[] = scoped.edges.map((edge) => ({
      id: edge.id,
      type: "lineage",
      source: edge.source,
      target: edge.target,
      data: edge.data,
      style: { stroke: "#374151" },
    }));
    return { nodes: flowNodes, edges: flowEdges };
  }, [state, affected, scope, selected]);

  if (state.status === "loading") {
    return <p className="text-xs text-slate-500" data-testid="lineage-graph-loading">Loading lineage graph…</p>;
  }
  if (state.status === "upgrade") {
    return <p className="text-xs text-amber-300" data-testid="lineage-graph-upgrade">{state.message}</p>;
  }
  if (state.status === "unavailable" || state.status === "error") {
    return <p className="text-xs text-slate-500" data-testid="lineage-graph-unavailable">{state.message}</p>;
  }
  if (state.graph.assets.length === 0) {
    return <p className="text-xs text-slate-500">No lineage assets are registered for this organization yet.</p>;
  }

  return (
    <div className="space-y-2" data-testid="lineage-graph">
      <div className="flex items-center gap-2 text-xs text-slate-500">
        <span>Scope</span>
        <div className="flex rounded-lg border border-[#374151] overflow-hidden">
          {(["contract", "org"] as const).map((value) => (
            <button
              key={value}
              type="button"
              onClick={() => setScope(value)}
              className={`px-2 py-1 ${scope === value ? "bg-[#1f2937] text-slate-100" : "text-slate-500"}`}
              data-testid={`lineage-scope-${value}`}
            >
              {value === "contract" ? "This contract" : "Whole org"}
            </button>
          ))}
        </div>
        <span className="ml-auto">
          {state.graph.asset_count} assets · {state.graph.edge_count} edges
        </span>
      </div>
      <div style={{ height: 420 }} className="rounded-xl border border-[#1f2937] overflow-hidden">
        <ReactFlowProvider>
          <ReactFlow
            nodes={nodes}
            edges={edges}
            nodeTypes={NODE_TYPES}
            edgeTypes={EDGE_TYPES}
            onNodeClick={(_, node) => setSelected((node as LineageNodeType).data)}
            onPaneClick={() => setSelected(null)}
            fitView
            minZoom={0.1}
            proOptions={{ hideAttribution: true }}
          >
            <Background color="#1f2937" />
            <Controls showInteractive={false} />
            <MiniMap pannable zoomable style={{ background: "#0a0d12" }} />
          </ReactFlow>
        </ReactFlowProvider>
      </div>
      {selected && (
        <div className="rounded-lg border border-[#1f2937] px-3 py-2 space-y-1" data-testid="lineage-node-details">
          <p className="text-sm text-slate-100">
            {selected.name} <span className="text-slate-500">({selected.kind}{selected.owner ? `, ${selected.owner}` : ""})</span>
          </p>
          {selected.path.length > 0 && (
            <p className="text-[11px] text-slate-500">{selected.path.join(" → ")}</p>
          )}
          {selected.changes.length > 0 ? (
            <ul className="space-y-1">
              {selected.changes.map((change) => (
                <li key={`${change.field}:${change.kind}`} className="text-xs text-slate-300 flex gap-2">
                  <span className={change.severity === "breaking" ? "text-rose-300" : "text-emerald-300"}>
                    {change.severity === "breaking" ? "Breaking" : "Safe"}
                  </span>
                  <span>{change.explanation}</span>
                </li>
              ))}
            </ul>
          ) : (
            <p className="text-xs text-slate-500">
              {selected.severity === "unknown"
                ? "Select a version above to see which field changes reach this asset."
                : "No field change from the selected version reaches this asset."}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
