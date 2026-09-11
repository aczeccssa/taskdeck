import type {ApiEnvelope} from "../api/client";

type Json = Record<string, unknown> | unknown[] | string | number | boolean | null;
type Stored = Record<string, Json[]>;

const ok = <T extends Json>(data: T, message = "ok"): ApiEnvelope<T> => ({ok: true, message, data});
const now = () => Date.now();
const id = (prefix: string) => `${prefix}-${crypto.randomUUID().slice(0, 8)}`;

/** Stateful same-origin mock. Its path and Response envelope match the daemon API. */
export function installMockApi(): void {
    const nativeFetch = window.fetch.bind(window);
    const store: Stored = {
        boards: [mockBoard()],
        workflows: [mockWorkflow()],
        dependencies: [],
        templates: [],
        quotas: [],
        tokens: [],
        rules: [mockRule()],
        policies: [mockPolicy()],
        mcpCalls: mockMcpCalls(),
        audits: mockAuditRecords(),
        notifications: [{
            id: "notice-1",
            event_type: "task_started",
            severity: "info",
            title: "Mock environment ready",
            message: "The web task is running and ready for inspection.",
            session: "mock-workspace",
            task: "web",
            read: false,
            created_at_ms: now()
        }],
    };
    const json = (data: Json, status = 200): Response => new Response(JSON.stringify(ok(data)), {
        status,
        headers: {"content-type": "application/json"}
    });
    let taskOrder = ["web", "worker"];
    let mockNodeName = "Mock device";
    let mockWorkspaceAlias = "Mock workspace";
    const pageQuery = new URLSearchParams(window.location.search);
    const taskStatus = pageQuery.get("taskStatus") ?? "running";
    const configFailure = pageQuery.get("configFailure");
    const metricsEmpty = pageQuery.get("metrics") === "empty";
    const apiFail = pageQuery.get("apiFail");
    const apiOffline = pageQuery.get("apiOffline") === "1";
    const apiDelay = Number(pageQuery.get("apiDelay") || 0);
    const errorJson = (message: string, data?: Json, status = 409): Response => new Response(JSON.stringify({ok: false, message, ...(data === undefined ? {} : {data})}), {
        status,
        headers: {"content-type": "application/json"}
    });
    const methodOf = (init?: RequestInit): string => (init?.method ?? "GET").toUpperCase();
    const bodyOf = async (init?: RequestInit): Promise<Record<string, unknown>> => {
        if (typeof init?.body !== "string" || !init.body) return {};
        try {
            return JSON.parse(init.body) as Record<string, unknown>;
        } catch {
            return {};
        }
    };
    const mutate = async (collection: keyof Stored, path: string, init?: RequestInit): Promise<Response> => {
        const method = methodOf(init);
        const body = await bodyOf(init);
        const itemId = path.split("/").filter(Boolean).at(-1);
        if (method === "GET") return json({[collection]: store[collection], targets: mockTargets()});
        if (method === "POST") {
            const item = {id: id(collection.slice(0, -1)), ...body, created_at_ms: now()};
            store[collection].push(item);
            return json(item, 201);
        }
        if (method === "PUT") {
            const item = store[collection].find((entry) => typeof entry === "object" && entry !== null && (entry as {
                id?: string
            }).id === itemId);
            if (item && typeof item === "object" && !Array.isArray(item)) Object.assign(item, body);
            return json(item ?? {id: itemId, ...body});
        }
        if (method === "DELETE") {
            store[collection] = store[collection].filter((entry) => !(typeof entry === "object" && entry !== null && (entry as {
                id?: string
            }).id === itemId));
            return json({id: itemId, deleted: true});
        }
        return json({});
    };

    const mockedFetch = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
        const url = new URL(typeof input === "string" ? input : input instanceof URL ? input.href : input.url, window.location.origin);
        if (!url.pathname.startsWith("/api/") && !["/me", "/healthz"].includes(url.pathname)) return nativeFetch(input, init);
        const path = url.pathname;
        const method = methodOf(init);
        if (apiOffline && path.startsWith("/api/")) throw new TypeError("Failed to fetch");
        if (apiDelay > 0) await new Promise((resolve) => setTimeout(resolve, apiDelay));
        if (apiFail && path.includes(apiFail)) return errorJson(`Mock failure for ${path}`, undefined, 500);
        if (path === "/healthz") return new Response("", {status: 200});
        if (path === "/me") return json({enabled: false, configured: false, authenticated: true});
        if (path === "/api/nodes") return json([mockNode(pageQuery.get("nodeState") !== "offline", mockNodeName)]);
        if (path === "/api/workspaces") return json([{
            session: "mock-workspace",
            alias: mockWorkspaceAlias,
            display_name: mockWorkspaceAlias,
            project: "/workspace/mock"
        }]);
        if (path === "/api/sessions") return json(["mock-workspace"]);
        if (path === "/api/node-metrics") return json(mockNodeMetrics(metricsEmpty));
        if (path === "/api/task-runs" || path === "/api/events") return json({
            items: mockTaskRuns(),
            page: 1,
            page_size: 20,
            total: 2,
            total_pages: 1,
            has_next: false,
            has_previous: false
        });
        if (path === "/api/mcp-calls") return json(page(store.mcpCalls));
        if (path === "/api/audit") return json(page(store.audits));
        if (path.startsWith("/api/mcp-calls/")) return json(findBy(store.mcpCalls, path));
        if (path.startsWith("/api/audit/")) return json(findBy(store.audits, path, "audit_id"));
        if (path === "/api/notifications") return json({
            notifications: store.notifications,
            unread_count: store.notifications.filter((n) => typeof n === "object" && n !== null && !(n as {
                read?: boolean
            }).read).length
        });
        if (path === "/api/notifications/read") {
            store.notifications.forEach((n) => {
                if (typeof n === "object" && n !== null) (n as { read: boolean }).read = true;
            });
            return json({marked: true});
        }
        if (path === "/api/action") return json({action: await bodyOf(init), accepted: true});
        if (path === "/api/workflow-groups" && method === "GET") return json({
            groups: store.workflows,
            targets: mockTargets(),
            ungrouped: []
        });
        if (path === "/api/notification-rules" && method === "GET") return json(store.rules);
        if (path === "/api/scaling-policies" && method === "GET") return json({
            policies: store.policies,
            targets: mockTargets()
        });
        if (path === "/api/board-templates" && method === "GET") return json({templates: store.templates});
        const templateExport = path.match(/^\/api\/board-templates\/([^/]+)\/export$/);
        if (templateExport) {
            const templateId = decodeURIComponent(templateExport[1]);
            const template = store.templates.find((entry) => typeof entry === "object" && entry !== null && (entry as {id?: string}).id === templateId);
            if (!template) return errorJson(`board template '${templateId}' not found`, undefined, 404);
            const record = template as Record<string, unknown>;
            return json({kind: "taskdeck_board_template", name: record.name ?? "Template", description: record.description ?? null, cards: Array.isArray(record.cards) ? record.cards : [], exported_at_ms: now()});
        }
        const templateApply = path.match(/^\/api\/board-templates\/([^/]+)\/apply$/);
        if (templateApply && method === "POST") {
            const templateId = decodeURIComponent(templateApply[1]);
            const template = store.templates.find((entry) => typeof entry === "object" && entry !== null && (entry as {id?: string}).id === templateId);
            if (!template) return errorJson(`board template '${templateId}' not found`, undefined, 404);
            const record = template as Record<string, unknown>;
            const body = await bodyOf(init);
            const board = {
                id: id("board"),
                name: String(body.name ?? `Board ${new Date().toISOString().slice(0, 10)}`),
                cards: (Array.isArray(record.cards) ? record.cards : []).map((card) => ({...(typeof card === "object" && card !== null ? card : {}), id: id("card")})),
                created_at_ms: now(),
                updated_at_ms: now()
            };
            store.boards.push(board);
            return json(board);
        }
        if (path === "/api/board-templates/import" && method === "POST") {
            const body = await bodyOf(init);
            if (body.kind !== "taskdeck_board_template") return errorJson("not a taskdeck board template export");
            const template = {id: id("template"), name: String(body.name ?? "Imported template"), description: body.description ?? null, cards: Array.isArray(body.cards) ? body.cards : [], created_at_ms: now(), updated_at_ms: now()};
            store.templates.push(template);
            return json(template, 201);
        }
        if (path === "/api/dependencies" && method === "POST") {
            const body = await bodyOf(init);
            const item = {
                id: id("dependency"),
                task_node_id: body.node_id ?? "",
                task_session: body.session ?? "",
                task: body.task ?? "",
                depends_node_id: body.depends_node_id ?? "",
                depends_session: body.depends_session ?? "",
                depends_task: body.depends_task ?? "",
                required_state: "running",
                task_status: "running",
                depends_status: "running",
                created_at_ms: now()
            };
            store.dependencies.push(item);
            return json(item, 201);
        }
        const simple = ({
            "/api/boards": "boards",
            "/api/workflow-groups": "workflows",
            "/api/dependencies": "dependencies",
            "/api/board-templates": "templates",
            "/api/quotas": "quotas",
            "/api/notification-rules": "rules",
            "/api/scaling-policies": "policies"
        } as Record<string, keyof Stored>)[path];
        if (simple) return mutate(simple, path, init);
        if (path === "/api/tokens") {
            if (method === "GET") return json({tokens: store.tokens});
            const body = await bodyOf(init);
            const token = {
                id: id("token"),
                name: String(body.name ?? "Mock token"),
                token_prefix: "tdk_mock",
                created_at_ms: now(),
                revoked: false
            };
            store.tokens.push(token);
            return json({...token, secret: "tdk_mock_example_secret"}, 201);
        }
        if (/^\/api\/sessions\/[^/]+$/.test(path)) return json(mockSnapshot(taskStatus, taskOrder));
        if (/\/logs$/.test(path)) {
            const after = Number(url.searchParams.get("after") || 0);
            const limit = Number(url.searchParams.get("limit") || 1000);
            const all = pageQuery.get("logs") === "long"
                ? Array.from({length: 160}, (_, index) => ({seq: index + 1, stream: index % 5 === 4 ? "stderr" : "stdout", text: `Mock output line ${index + 1} with searchable token`}))
                : [{seq: 1, stream: "stdout", text: "Mock Taskdeck is running."}, {seq: 2, stream: "stderr", text: "Watching mock output."}];
            return json({generation: 1, reset: false, lines: all.filter((line) => Number(line.seq) > after).slice(-limit)});
        }
        if (/\/history$/.test(path)) return json({accepted: true});
        if (/\/metrics$/.test(path)) {
            const timestamp = now();
            if (metricsEmpty) {
                return json({
                    sample_interval_ms: 1000,
                    window_seconds: 600,
                    cpu_percent_unit: "percent",
                    running: false,
                    current: {cpu_percent: 0, memory_bytes: 0, process_count: 0},
                    samples: [],
                    processes: [],
                    restart_markers_ms: []
                });
            }
            return json({
                sample_interval_ms: 1000,
                window_seconds: 600,
                cpu_percent_unit: "percent",
                running: taskStatus === "running",
                current: {cpu_percent: 12, memory_bytes: 256000000, process_count: 2},
                samples: [{timestamp_ms: timestamp - 1000, cpu_percent: 10, memory_bytes: 250000000, process_count: 2}, {timestamp_ms: timestamp, cpu_percent: 12, memory_bytes: 256000000, process_count: 2}],
                processes: [{pid: 1234, ppid: 1, name: "bun", status: "running", run_time_seconds: 42, cpu_percent: 10, memory_bytes: 128000000, process_count: 1}, {pid: 1235, ppid: 1234, name: "vite", status: "running", run_time_seconds: 40, cpu_percent: 2, memory_bytes: 128000000, process_count: 1}],
                restart_markers_ms: [timestamp - 500]
            });
        }
        if (/\/config$/.test(path)) {
            if (method === "PUT" && configFailure === "stale_revision") {
                return errorJson("configuration revision is stale", {kind: "stale_revision"});
            }
            if (method === "PUT" && configFailure === "reconciliation_error") {
                return errorJson("one or more live sessions failed to reconcile", {kind: "reconciliation_error", saved: true, current_revision: "recovered-1"});
            }
            if (method === "PUT") {
                const body = await bodyOf(init);
                const tasks = Array.isArray(body.tasks) ? body.tasks as Json[] : [];
                taskOrder = tasks.map((task) => String((task as Record<string, unknown>).label));
                return json({...mockConfig(), revision: `mock-${now()}`, workspace_env: body.workspace_env ?? {}, tasks});
            }
            return json(mockConfig());
        }
        if (/\/settings$/.test(path)) {
            if (method === "PUT") {
                const body = await bodyOf(init);
                if (path === "/api/nodes/mock/settings" && typeof body.name === "string" && body.name.trim()) mockNodeName = body.name.trim();
            }
            return json({settings: mockNode(pageQuery.get("nodeState") !== "offline", mockNodeName), environment_overrides: []});
        }
        if (/^\/api\/workspaces\/[^/]+\/alias$/.test(path)) {
            if (method === "PUT") {
                const body = await bodyOf(init);
                mockWorkspaceAlias = typeof body.alias === "string" ? body.alias.trim() : "";
            }
            return json({session: "mock-workspace", alias: mockWorkspaceAlias, display_name: mockWorkspaceAlias});
        }
        if (/\/service$/.test(path)) return json({status: "running", supported: true});
        if (/\/revisions$/.test(path)) return json({revisions: []});
        if (/\/run$|\/actions$|\/restore$|\/apply$|\/import$/.test(path)) return json({accepted: true});
        const match = path.match(/^\/api\/(boards|workflow-groups|dependencies|board-templates|quotas|tokens|notification-rules|scaling-policies)\//);
        if (match) {
            const collection = ({
                boards: "boards",
                "workflow-groups": "workflows",
                dependencies: "dependencies",
                "board-templates": "templates",
                quotas: "quotas",
                tokens: "tokens",
                "notification-rules": "rules",
                "scaling-policies": "policies"
            } as Record<string, keyof Stored>)[match[1]];
            return mutate(collection, path, init);
        }
        return json({});
    };
    window.fetch = mockedFetch as typeof window.fetch;
}

function page(items: Json[]): Json {
    return {items, page: 1, page_size: 20, total: items.length, total_pages: 1, has_next: false, has_previous: false};
}

function findBy(items: Json[], path: string, key = "id"): Json {
    const value = decodeURIComponent(path.split("/").at(-1) ?? "");
    return items.find((item) => typeof item === "object" && item !== null && (item as Record<string, unknown>)[key] === value) ?? {};
}

function mockNode(online = true, name = "Mock device"): Json {
    return {
        id: "mock",
        node_id: "mock",
        name,
        is_self: true,
        online,
        role: "leader",
        mode: "standard",
        leader_mode: "standard",
        leader_url: "http://leader:9837",
        bind_host: "127.0.0.1",
        web_port: 9837,
        enrollment_token: null,
        sessions: ["mock-workspace"],
        last_seen_ms: now()
    };
}

function mockNodeMetrics(empty = false): Json {
    const timestamp = now();
    const sample = {
        timestamp_ms: timestamp,
        cpu_percent: 12,
        memory_bytes: 256000000,
        memory_total_bytes: 1073741824,
        running_tasks: 1
    };
    if (empty) {
        return {
            nodes: [{
                node_id: "mock",
                node_name: "Mock device",
                online: true,
                is_self: true,
                current: null,
                samples: [],
                session_count: 1,
                task_status_counts: {running: 1, stopped: 1}
            }], task_status_counts: {running: 1, stopped: 1}
        };
    }
    return {
        nodes: [{
            node_id: "mock",
            node_name: "Mock device",
            online: true,
            is_self: true,
            current: sample,
            samples: [{...sample, timestamp_ms: timestamp - 60000, cpu_percent: 9}, sample],
            session_count: 1,
            task_status_counts: {running: 1, stopped: 1}
        }], task_status_counts: {running: 1, stopped: 1}
    };
}

function mockTargets(): Json[] {
    return [{
        node_id: "mock",
        node_name: "Mock device",
        session: "mock-workspace",
        workspace_display_name: "Mock workspace",
        task: "web",
        tasks: ["web", "worker"],
        label: "Mock web"
    }];
}

function mockBoard(): Json {
    return {
        id: "board-delivery",
        name: "Delivery readiness",
        cards: [{
            id: "card-web",
            node_id: "mock",
            node_name: "Mock device",
            session: "mock-workspace",
            workspace_display_name: "Mock workspace",
            task: "web",
            mode: "status",
            pinned: true
        }, {
            id: "card-worker",
            node_id: "mock",
            node_name: "Mock device",
            session: "mock-workspace",
            workspace_display_name: "Mock workspace",
            task: "worker",
            mode: "metrics",
            pinned: false
        }]
    };
}

function mockWorkflow(): Json {
    return {
        id: "workflow-release",
        name: "Release path",
        members: [{
            node_id: "mock",
            node_name: "Mock device",
            session: "mock-workspace",
            workspace_display_name: "Mock workspace",
            task: "web",
            available: true
        }],
        graph: {positions: [{x: 40, y: 30}], edges: []}
    };
}

function mockRule(): Json {
    return {
        id: "rule-failures",
        name: "Web task failures",
        event_types: ["task_failed", "task_exited"],
        enabled: true,
        scope_session: "mock-workspace",
        scope_task: "web",
        webhook_url: null,
        created_at_ms: now(),
        updated_at_ms: now()
    };
}

function mockPolicy(): Json {
    return {
        id: "policy-web",
        name: "Web capacity",
        enabled: true,
        watch_node_id: "mock",
        watch_session: "mock-workspace",
        watch_task: "web",
        metric: "cpu_percent",
        scale_out_threshold: 75,
        scale_in_threshold: 20,
        scale_out_node_id: "mock",
        scale_out_session: "mock-workspace",
        scale_out_task: "worker",
        cooldown_seconds: 300,
        created_at_ms: now(),
        updated_at_ms: now()
    };
}

function mockMcpCalls(): Json[] {
    return [{
        id: "call-001",
        operation: "sessions",
        tool: "taskdeck.sessions",
        input: {},
        target_node: "mock",
        success: true,
        duration_ms: 42,
        started_at_ms: now() - 90000,
        request: {id: "mock-request-001", params: {arguments: {}}},
        response: {sessions: ["mock-workspace"]}
    }, {
        id: "call-002",
        operation: "logs",
        tool: "taskdeck.logs",
        input: {session: "mock-workspace", task: "web", tail: 100},
        target_node: "mock",
        success: true,
        duration_ms: 18,
        started_at_ms: now() - 45000,
        request: {id: "mock-request-002", params: {arguments: {session: "mock-workspace", task: "web"}}},
        response: {lines: 1}
    }, {
        id: "call-003",
        operation: "restart",
        tool: "taskdeck.restart",
        input: {session: "mock-workspace", task: "worker"},
        target_node: "mock",
        success: false,
        duration_ms: 103,
        started_at_ms: now() - 15000,
        request: {id: "mock-request-003", params: {arguments: {session: "mock-workspace", task: "worker"}}},
        response: {message: "Worker is stopped"},
        error: "Worker is stopped"
    }];
}

function mockAuditRecords(): Json[] {
    return [{
        audit_id: "audit-001",
        timestamp_ms: now() - 90000,
        source: "web",
        transport: "http",
        origin_node_id: "mock",
        executor_node_id: "mock",
        operation: "mcp_sessions",
        request_kind: "mcp",
        session: "mock-workspace",
        status: "success",
        success: true,
        duration_ms: 42,
        replicated_at_ms: now() - 89000,
        correlation_id: "mock-request-001",
        request: {operation: "sessions"},
        response: {count: 1},
        details: {mock: true}
    }, {
        audit_id: "audit-002",
        timestamp_ms: now() - 45000,
        source: "mcp",
        transport: "http",
        origin_node_id: "mock",
        executor_node_id: "mock",
        operation: "task_logs",
        request_kind: "mcp",
        session: "mock-workspace",
        task: "web",
        status: "success",
        success: true,
        duration_ms: 18,
        replicated_at_ms: now() - 44000,
        correlation_id: "mock-request-002",
        request: {tail: 100},
        response: {lines: 1},
        details: {mock: true}
    }, {
        audit_id: "audit-003",
        timestamp_ms: now() - 15000,
        source: "web",
        transport: "http",
        origin_node_id: "mock",
        executor_node_id: "mock",
        operation: "task_restart",
        request_kind: "action",
        session: "mock-workspace",
        task: "worker",
        status: "error",
        success: false,
        duration_ms: 103,
        correlation_id: "mock-request-003",
        request: {action: "restart"},
        response: {message: "Worker is stopped"},
        error: "Worker is stopped",
        details: {mock: true}
    }];
}

function mockTaskRuns(): Json[] {
    return [{id: "run-001", task: "web", session: "mock-workspace", status: "running"}, {
        id: "run-002",
        task: "worker",
        session: "mock-workspace",
        status: "stopped"
    }];
}

function mockConfig(): Record<string, unknown> {
    return {
        session: "mock-workspace",
        project: "/workspace/mock",
        source: "mock",
        revision: "mock-1",
        workspace_env: {MOCK_ENV: "1"},
        tasks: [
            {label: "web", command: "bun run dev", args: ["--host"], cwd: ".", env: {NODE_ENV: "development"}, shell: true, auto_start: true, stop_timeout_ms: 3000, clear_logs_on_restart: false, schedule: null, origin: {imported: false, has_yaml_override: false}},
            {label: "worker", command: "bun run worker", args: [], cwd: ".", env: {}, shell: true, auto_start: false, stop_timeout_ms: 5000, clear_logs_on_restart: true, schedule: null, origin: {imported: false, has_yaml_override: false}}
        ]
    };
}

function mockSnapshot(status = "running", order: string[] = ["web", "worker"]): Json {
    return {
        name: "mock-workspace",
        project: "/workspace/mock",
        source: "mock",
        alias: "Mock workspace",
        task_order: order,
        tasks: {
            web: {
                label: "web",
                status,
                pid: 1234,
                command: "bun run dev",
                cwd: "/workspace/mock",
                auto_start: true,
                logs: [],
                run_generation: 1,
                started_at_ms: now(),
                service: {
                    classification: "service",
                    technology: {runtime: "Bun", framework: "Vite", confidence: "high", evidence: []},
                    endpoints: [],
                    inspection: "listening"
                }
            },
            worker: {
                label: "worker",
                status: "idle",
                pid: null,
                command: "bun run worker",
                cwd: "/workspace/mock",
                auto_start: false,
                logs: [],
                run_generation: 0,
                started_at_ms: 0,
                service: {
                    classification: "process",
                    technology: {confidence: "unknown", evidence: []},
                    endpoints: [],
                    inspection: "not_running"
                }
            }
        }
    };
}
