import { expect, test } from "bun:test";
import { keepAvailable } from "./selection";

test("selection survives polling and route reload while its target exists", () => {
    expect(keepAvailable("workspace-b", ["workspace-a", "workspace-b"], "workspace-a")).toBe("workspace-b");
});

test("selection falls back only after the current target disappears", () => {
    expect(keepAvailable("missing", ["workspace-a", "workspace-b"], "workspace-b")).toBe("workspace-b");
    expect(keepAvailable("missing", ["workspace-a", "workspace-b"], "also-missing")).toBe("workspace-a");
    expect(keepAvailable("missing", [])).toBe("");
});
