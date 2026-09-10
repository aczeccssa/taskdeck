import { useEffect, type RefObject } from "react";
import type { ConfigTaskDraft } from "./helpers";
import type { SortDrag } from "./types";

export function useConfigTaskDrag({
    configListRef,
    sortDrag,
    tasks,
    taskIndex,
    setTasks,
    setTaskIndex,
    setDirty,
}: {
    configListRef: RefObject<HTMLElement | null>;
    sortDrag: { current: SortDrag | null };
    tasks: ConfigTaskDraft[];
    taskIndex: number;
    setTasks: React.Dispatch<React.SetStateAction<ConfigTaskDraft[]>>;
    setTaskIndex: React.Dispatch<React.SetStateAction<number>>;
    setDirty: React.Dispatch<React.SetStateAction<boolean>>;
}) {
    useEffect(() => {
        const container = configListRef.current;
        if (!container) return;
        const readOrder = (): string[] =>
            Array.from(container.querySelectorAll<HTMLElement>(".config-task-item")).map(
                (element) => element.dataset.orderKey ?? "",
            );
        const finish = (event: PointerEvent): void => {
            const drag = sortDrag.current;
            if (!drag || drag.pointerId !== event.pointerId) return;
            drag.element.classList.remove("dragging");
            if (drag.active) {
                const order = readOrder();
                if (order.join("\u0000") !== drag.original.join("\u0000")) {
                    const selectedKey = tasks[taskIndex]?._key;
                    const byKey = new Map(tasks.map((task) => [task._key, task]));
                    const next = order
                        .map((key) => byKey.get(key))
                        .filter((task): task is ConfigTaskDraft => Boolean(task));
                    setTasks(next);
                    setDirty(true);
                    if (selectedKey)
                        setTaskIndex(
                            Math.max(
                                0,
                                next.findIndex((task) => task._key === selectedKey),
                            ),
                        );
                }
            }
            sortDrag.current = null;
        };
        const onPointerDown = (event: PointerEvent): void => {
            if (event.button !== 0) return;
            const target = event.target instanceof Element ? event.target : null;
            const handle = target?.closest<HTMLElement>("[data-drag-handle]");
            if (!handle) return;
            const item = target?.closest<HTMLElement>(".config-task-item");
            const key = item?.dataset.orderKey;
            if (!item || !key) return;
            item.setPointerCapture?.(event.pointerId);
            item.classList.add("dragging");
            sortDrag.current = {
                pointerId: event.pointerId,
                key,
                element: item,
                startX: event.clientX,
                startY: event.clientY,
                active: false,
                original: readOrder(),
            };
        };
        const onPointerMove = (event: PointerEvent): void => {
            const drag = sortDrag.current;
            if (!drag || drag.pointerId !== event.pointerId) return;
            if (!drag.active && Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY) < 6) return;
            event.preventDefault();
            drag.active = true;
            drag.element.classList.add("dragging");
            const target = document
                .elementFromPoint(event.clientX, event.clientY)
                ?.closest<HTMLElement>(".config-task-item");
            const targetKey = target?.dataset.orderKey;
            if (!targetKey || targetKey === drag.key) return;
            const rect = target?.getBoundingClientRect();
            if (!rect) return;
            const before = event.clientY < rect.top + rect.height / 2;
            container.insertBefore(drag.element, before ? target : target.nextSibling);
        };
        container.addEventListener("pointerdown", onPointerDown);
        container.addEventListener("pointermove", onPointerMove);
        container.addEventListener("pointerup", finish);
        container.addEventListener("pointercancel", finish);
        return () => {
            container.removeEventListener("pointerdown", onPointerDown);
            container.removeEventListener("pointermove", onPointerMove);
            container.removeEventListener("pointerup", finish);
            container.removeEventListener("pointercancel", finish);
        };
    }, [tasks, taskIndex]);

}
