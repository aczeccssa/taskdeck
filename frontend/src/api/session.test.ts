import { describe, expect, test } from "bun:test";
import { configSaveFailure } from "./session";

describe("session config save failure", () => {
    test("classifies configuration save response kinds", () => {
        expect(configSaveFailure({ kind: "stale_revision" })).toEqual({ kind: "stale_revision", saved: undefined, current_revision: undefined });
        expect(configSaveFailure({ kind: "reconciliation_error", saved: true, current_revision: "rev-2" })).toEqual({ kind: "reconciliation_error", saved: true, current_revision: "rev-2" });
        expect(configSaveFailure({ kind: "validation_error" })).toBeUndefined();
        expect(configSaveFailure(null)).toBeUndefined();
    });
});
