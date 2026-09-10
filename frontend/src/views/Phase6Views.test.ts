import {describe, expect, test} from "bun:test";
import {normalizedGraph, reorder} from "./Phase6Views";

describe("phase 6 workflow helpers", () => {
    test("normalizes missing positions and removes invalid graph edges", () => {
        const graph = normalizedGraph({positions: [{x: 10, y: 20}], edges: [{from: 0, to: 1}, {from: -1, to: 0}, {from: 0, to: 4}]}, 2);
        expect(graph.positions).toEqual([{x: 10, y: 20}, {x: 270, y: 30}]);
        expect(graph.edges).toEqual([{from: 0, to: 1}]);
    });
    test("reorders members without mutating the source", () => {
        const source = ["a", "b", "c"];
        expect(reorder(source, 2, 0)).toEqual(["c", "a", "b"]);
        expect(source).toEqual(["a", "b", "c"]);
        expect(reorder(source, 0, -1)).toBe(source);
    });
});
