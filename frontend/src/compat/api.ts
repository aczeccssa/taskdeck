import {BoundaryError, booleanValue, field, isRecord, optional, stringValue, type Decoder} from "../lib/narrow";
export interface LegacyEnvelope<T> { ok: boolean; message: string; data?: T }
export function envelopeOf<T>(decodeData: Decoder<T>): Decoder<LegacyEnvelope<T>> {
 return (value, path = "response") => {
  if (!isRecord(value)) throw new BoundaryError(`${path} must be an object`);
  const data = optional(decodeData)(value.data, `${path}.data`);
  return {ok: field(value, "ok", booleanValue, path), message: field(value, "message", stringValue, path), ...(data === undefined ? {} : {data})};
 };
}
export interface LegacyApiDependencies { fetch: (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>; redirectToLogin: () => void }
const defaults: LegacyApiDependencies = {fetch: (input, init) => globalThis.fetch(input, init), redirectToLogin: () => window.location.assign("/login")};
/** Compatibility boundary matching legacy requestJson: only 401 redirects/throws; ok:false envelopes are returned. */
export class LegacyApiAdapter {
 constructor(private readonly dependencies: LegacyApiDependencies = defaults) {}
 async request<T>(url: string, decodeData: Decoder<T>, options?: RequestInit): Promise<LegacyEnvelope<T>> {
 const response = await this.dependencies.fetch(url, options);
 if (response.status === 401) { this.dependencies.redirectToLogin(); throw new Error("Authentication required"); }
  const text = await response.text();
  let value: unknown;
  try { value = JSON.parse(text); }
  catch {
   if (!response.ok) return {ok: false, message: text.trim() || `Request failed (${response.status})`};
   throw new BoundaryError("response must be valid JSON");
  }
  return envelopeOf(decodeData)(value);
 }
}
