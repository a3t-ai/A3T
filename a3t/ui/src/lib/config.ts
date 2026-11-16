// Backend URL resolution.
//
// The API host is derived at runtime from where the app is served (one build,
// all environments), with two overrides:
//   1. VITE_API_URL  — a build-time pin (used for local dev).
//   2. localStorage 'a3t.apiBase' — a runtime switch set from Settings, so an
//      operator can flip the console between the CE and EE runtimes to test.
//
//   dash.dev.a3t.io  ->  https://console.dev.a3t.io
//   dash.a3t.io      ->  https://console.a3t.io

const KNOWN_DOMAINS = ['a3t.io', 'a3tplatform.com'];
const OVERRIDE_KEY = 'a3t.apiBase';

// Hosts served by another backend than their name says. Empty: since 16 Nov 2026
// console.a3t.io is the prod cluster with the current image, so dash.a3t.io uses
// it (the UAT pin to console.dev.a3t.io is gone). The Settings runtime switch
// still wins over this table.
const HOST_BACKEND_PIN: Record<string, string> = {};

/** Hostname-derived backend, ignoring any override. */
function hostnameApiBase(): string {
	if (typeof window === 'undefined') return '';
	const host = window.location.hostname;
	if (HOST_BACKEND_PIN[host]) return HOST_BACKEND_PIN[host];
	const domain = KNOWN_DOMAINS.find((d) => host === d || host.endsWith('.' + d));
	if (!domain) return ''; // unknown host -> same-origin
	const isDev = host === 'dev.' + domain || host.endsWith('.dev.' + domain);
	return `https://console.${isDev ? 'dev.' : ''}${domain}`;
}

/** The default backend (build pin or hostname), ignoring the runtime override. */
export function getDefaultApiBase(): string {
	return (import.meta.env.VITE_API_URL as string) || hostnameApiBase();
}

export function getApiOverride(): string | null {
	if (typeof window === 'undefined') return null;
	try {
		return localStorage.getItem(OVERRIDE_KEY);
	} catch {
		return null;
	}
}

export function setApiOverride(url: string) {
	try {
		localStorage.setItem(OVERRIDE_KEY, url);
	} catch {}
}

export function clearApiOverride() {
	try {
		localStorage.removeItem(OVERRIDE_KEY);
	} catch {}
}

function resolveApiBase(): string {
	const buildPin = import.meta.env.VITE_API_URL as string | undefined;
	if (buildPin) return buildPin;
	if (typeof window === 'undefined') return '';
	const runtime = getApiOverride(); // Settings runtime switch wins over hostname
	if (runtime) return runtime;
	return hostnameApiBase();
}

export const API_BASE = resolveApiBase();
