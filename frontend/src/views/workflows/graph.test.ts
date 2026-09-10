import { describe, expect, test } from "bun:test";
import { normalizedGraph } from "./graph";

describe("workflow graph helpers", () => {
    test("normalizes missing positions and removes invalid graph edges", () => {
        const graph = normalizedGraph({ positions: [{ x: 10, y: 20 }], edges: [{ from: 0, to: 1 }, { from: -1, to: 0 }, { from: 0, to: 4 }] }, 2);
        expect(graph.positions).toEqual([{ x: 10, y: 20 }, { x: 270, y: 30 }]);
        expect(graph.edges).toEqual([{ from: 0, to: 1 }]);
    });
});
