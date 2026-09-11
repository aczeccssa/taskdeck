import { expect, test } from "bun:test";
import { scalingPayload, type ScalingPolicyDraft } from "./ScalingSettings";

const draft: ScalingPolicyDraft = {name: " API load ", enabled: true, watch: "node-a\u0000api\u0000server", metric: "cpu_percent", scale_out_threshold: "80", scale_in_threshold: "25", scale_out: "node-b\u0000workers\u0000worker", cooldown_seconds: "300"};

test("scaling policy payload includes both complete task targets", () => {
    expect(scalingPayload(draft)).toEqual({name: "API load", enabled: true, metric: "cpu_percent", scale_out_threshold: 80, scale_in_threshold: 25, cooldown_seconds: 300, watch_node_id: "node-a", watch_session: "api", watch_task: "server", scale_out_node_id: "node-b", scale_out_session: "workers", scale_out_task: "worker"});
});

test("scaling policy rejects inverted thresholds and missing targets", () => {
    expect(() => scalingPayload({...draft, scale_in_threshold: "90"})).toThrow("lower threshold");
    expect(() => scalingPayload({...draft, scale_out: ""})).toThrow("action target");
});
