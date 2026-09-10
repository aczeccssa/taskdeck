import type {JsonValue} from "../domain/models";
export type Decoder<T> = (value: unknown, path?: string) => T;
export class BoundaryError extends Error { constructor(message: string) { super(message); this.name = "BoundaryError"; } }
export function isRecord(value: unknown): value is Record<string, unknown> { return typeof value === "object" && value !== null && !Array.isArray(value); }
export const stringValue: Decoder<string> = (value, path = "value") => { if (typeof value !== "string") throw new BoundaryError(`${path} must be a string`); return value; };
export const numberValue: Decoder<number> = (value, path = "value") => { if (typeof value !== "number" || !Number.isFinite(value)) throw new BoundaryError(`${path} must be a finite number`); return value; };
export const booleanValue: Decoder<boolean> = (value, path = "value") => { if (typeof value !== "boolean") throw new BoundaryError(`${path} must be a boolean`); return value; };
export function arrayOf<T>(decode: Decoder<T>): Decoder<T[]> { return (value, path = "value") => { if (!Array.isArray(value)) throw new BoundaryError(`${path} must be an array`); return value.map((item, index) => decode(item, `${path}[${index}]`)); }; }
export function optional<T>(decode: Decoder<T>): Decoder<T | undefined> { return (value, path) => value === undefined ? undefined : decode(value, path); }
export function nullable<T>(decode: Decoder<T>): Decoder<T | null> { return (value, path) => value === null ? null : decode(value, path); }
export function field<T>(record: Record<string, unknown>, key: string, decode: Decoder<T>, path = "value"): T { return decode(record[key], `${path}.${key}`); }
export const jsonValue: Decoder<JsonValue> = (value, path = "value") => {
 if (value === null || typeof value === "string" || typeof value === "boolean") return value;
 if (typeof value === "number" && Number.isFinite(value)) return value;
 if (Array.isArray(value)) return value.map((item, index) => jsonValue(item, `${path}[${index}]`));
 if (isRecord(value)) { const result: Record<string, JsonValue> = {}; for (const [key, item] of Object.entries(value)) result[key] = jsonValue(item, `${path}.${key}`); return result; }
 throw new BoundaryError(`${path} is not valid JSON data`);
};
