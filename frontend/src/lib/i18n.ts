import type {Language} from "../domain/state";
import {preferenceKey, parseStoredLanguage} from "./helpers";

type MessageTable = Readonly<Record<string, string>>;

/** Shell-level strings only. View-body copy remains English; see the migration log deviation note. */
const en: MessageTable = {
    "nav.tasks": "Tasks",
    "nav.dashboard": "Dashboard",
    "nav.workflows": "Workflows",
    "nav.boards": "Boards",
    "nav.alerts": "Alerts",
    "nav.calls": "MCP Calls",
    "nav.audit": "Audit Log",
    "nav.docs": "MCP Guide",
    "nav.settings": "Settings",
    "view.tasks": "Task workspace",
    "view.dashboard": "Dashboard",
    "view.workflows": "Workflows",
    "view.boards": "Boards",
    "view.alerts": "Alerts",
    "view.calls": "MCP Calls",
    "view.audit": "Audit Log",
    "view.docs": "MCP Guide",
    "view.settings": "Settings",
};

const zh: MessageTable = {
    "nav.tasks": "任务",
    "nav.dashboard": "仪表盘",
    "nav.workflows": "工作流",
    "nav.boards": "看板",
    "nav.alerts": "告警",
    "nav.calls": "MCP 调用",
    "nav.audit": "审计日志",
    "nav.docs": "MCP 指南",
    "nav.settings": "设置",
    "view.tasks": "任务工作区",
    "view.dashboard": "仪表盘",
    "view.workflows": "工作流",
    "view.boards": "看板",
    "view.alerts": "告警",
    "view.calls": "MCP 调用",
    "view.audit": "审计日志",
    "view.docs": "MCP 指南",
    "view.settings": "设置",
};

const tables: Readonly<Record<Language, MessageTable>> = {en, zh};

export function storedLanguage(): Language {
    return parseStoredLanguage(localStorage.getItem(preferenceKey("lang")));
}

export function viewTitle(view: string, language: Language): string {
    return tables[language][`view.${view}`] ?? tables.en[`view.${view}`] ?? view;
}

/** Mirrors legacy applyI18n: swap [data-i18n] text, the lang button label, and the document lang. */
export function applyI18n(language: Language): void {
    const table = tables[language];
    document.querySelectorAll<HTMLElement>("[data-i18n]").forEach((element) => {
        const value = element.dataset.i18n ? table[element.dataset.i18n] : undefined;
        if (value != null) element.textContent = value;
    });
    const langButton = document.getElementById("lang");
    if (langButton) langButton.textContent = language === "en" ? "EN" : "中";
    document.documentElement.lang = language === "zh" ? "zh-CN" : "en";
}
