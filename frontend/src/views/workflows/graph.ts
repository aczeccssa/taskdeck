import type { WorkflowGraph, WorkflowTargetView } from "../../domain/models";

export const emptyGraph = (): WorkflowGraph => ({ positions: [], edges: [] });
export const targetKey = (target: WorkflowTargetView): string => `${target.node_id}\0${target.session}`;

export function normalizedGraph(graph: WorkflowGraph, count: number): WorkflowGraph {
    return {
        positions: Array.from({ length: count }, (_, i) => graph.positions[i] ?? { x: 40 + (i % 3) * 230, y: 30 + Math.floor(i / 3) * 110 }),
        edges: graph.edges.filter((edge) => edge.from >= 0 && edge.to >= 0 && edge.from < count && edge.to < count),
    };
}
