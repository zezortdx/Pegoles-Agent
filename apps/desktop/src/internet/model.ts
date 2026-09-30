import type { EgressDecision, InternetAccess, InternetMode, InternetStatus } from "../lib/tauri";

/** Core's allowlist limit (docs/EGRESS.md). */
export const MAX_SITES = 32;

/** The composer's choice for the next task. Always starts (and returns to) off. */
export interface InternetDraft {
  readonly mode: InternetMode;
  readonly sites: readonly string[];
}

export const INTERNET_OFF: InternetDraft = { mode: "off", sites: [] };

export const NO_INTERNET: InternetStatus = {
  active: false, task_id: null, mode: "off", domains: [], allowed: 0, blocked: 0, dropped: 0, recent: [],
};

export function toAccess(draft: InternetDraft): InternetAccess {
  return { mode: draft.mode, domains: draft.mode === "allowlist" ? [...draft.sites] : [] };
}

/** Why this choice can't be handed over yet, or null. */
export function draftProblem(draft: InternetDraft): string | null {
  return draft.mode === "allowlist" && draft.sites.length === 0
    ? "Add at least one site, or turn Internet off for this task."
    : null;
}

export type SiteResult = { readonly site: string } | { readonly problem: string };

const IPV4 = /^\d{1,3}(\.\d{1,3}){3}$/;

/**
 * A friendly first pass over one typed or pasted site. Core validates
 * again (registrable names, no IPs, no wildcards, max 32) and is the
 * authority; this only saves a round trip and explains in plain words.
 */
export function parseSite(raw: string): SiteResult {
  let text = raw.trim().toLowerCase();
  text = text.replace(/^[a-z][a-z0-9+.-]*:\/\//, "");
  text = text.split(/[/?#]/, 1)[0].replace(/\.$/, "");
  if (text === "") return { problem: "Type a site name, like example.com." };
  if (text.includes("*")) return { problem: "No wildcards needed: a site already covers its subdomains." };
  if (text.includes("@") || text.includes(":") || /\s/u.test(text)) return { problem: "Type just the site name, like example.com." };
  if (IPV4.test(text)) return { problem: "Use a site name, not an IP address." };
  if (!text.includes(".") || text.startsWith(".") || text.includes("..")) return { problem: "That doesn’t look like a site name. Try example.com." };
  return { site: text };
}

/** Split pasted text into candidate sites. */
export function splitSites(raw: string): string[] {
  return raw.split(/[\s,;]+/u).filter((part) => part.length > 0);
}

/** One phrase per Core reason code (docs/EGRESS.md), in plain words. */
const REASONS: Readonly<Record<string, string>> = {
  allowed: "allowed",
  transport_method: "request type not allowed",
  transport_port: "only standard web ports are allowed",
  transport_scheme: "only web addresses are allowed",
  transport_userinfo: "the address contains a login",
  transport_ip_literal: "numeric addresses are not allowed",
  transport_no_dot: "not a public site name",
  transport_reserved_name: "private or internal name",
  transport_invalid_host: "not a valid site name",
  transport_mixed_script: "look-alike site name",
  transport_malformed: "malformed request",
  transport_head_too_large: "request too large",
  transport_host_mismatch: "request did not match its site",
  resolve_failed: "site could not be found",
  resolve_non_global: "points to a private network address",
  threat_malware: "known malware site",
  threat_phishing: "known phishing site",
  threat_url: "known harmful page",
  category_adult: "adult site",
  category_gambling: "gambling site",
  mode_not_in_allowlist: "not one of the allowed sites",
  inspect_blocked_signature: "program or archive download",
  inspect_download_not_safe: "download type not allowed",
  inspect_download_too_large: "download too large",
  inspect_encoded_body: "could not be checked safely",
  limit_body_too_large: "upload too large",
  limit_session_bytes: "this task reached its data limit",
  limit_streams: "too many connections at once",
  upstream_connect: "could not connect",
  upstream_tls: "the site’s certificate did not verify",
  upstream_protocol: "unexpected answer from the site",
  limit_idle: "the connection went quiet",
};

export type Verdict = "allowed" | "blocked" | "failed";

export interface DecisionLine {
  readonly verdict: Verdict;
  /** "Allowed", "Blocked" or "Failed". */
  readonly label: string;
  /** The reason in plain language, e.g. "download type not allowed". */
  readonly text: string;
  readonly host: string;
}

const FAILED = new Set(["upstream_connect", "upstream_protocol", "limit_idle", "resolve_failed"]);

export function decisionLine(decision: EgressDecision): DecisionLine {
  const text = REASONS[decision.reason] ?? "not allowed";
  const verdict: Verdict = decision.allowed ? "allowed" : FAILED.has(decision.reason) ? "failed" : "blocked";
  const label = verdict === "allowed" ? "Allowed" : verdict === "failed" ? "Failed" : "Blocked";
  return { verdict, label, text, host: decision.host || "unknown site" };
}

/** Newest first, at most `limit`. */
export function latest(decisions: readonly EgressDecision[], limit: number): EgressDecision[] {
  return decisions.slice(-limit).reverse();
}

export function scopeText(mode: InternetMode, sites: readonly string[]): string {
  if (mode === "open_web") return "Open web";
  if (mode === "allowlist") return `Only these sites: ${sites.join(", ")}`;
  return "Off";
}

/** The composer chip's label. */
export function chipLabel(draft: InternetDraft): string {
  if (draft.mode === "open_web") return "Open web";
  if (draft.mode === "allowlist") return draft.sites.length === 1 ? "1 site" : `${draft.sites.length} sites`;
  return "Off";
}
