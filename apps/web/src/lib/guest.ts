import { getApiConfig } from "./config";

const TIMEOUT_MS = 8000;

/**
 * Response of `POST /v1/guest` in `services/core`.
 *
 * Temporary hand-written type, matching the health.ts pattern: replace it with the type from
 * `packages/sdk-ts` (`GuestSessionOut`, generated from the OpenAPI schema) once this app depends
 * on that package. The full guest-driven judge UI is Martin's G6; this helper is the primitive it
 * builds on and proves the login-free path reaches the API from the browser.
 */
export interface GuestSession {
  token: string;
  role: "guest";
  sandbox_id: string;
  expires_at: string;
}

export interface GuestSuccess {
  kind: "ok";
  session: GuestSession;
}

export interface GuestFailure {
  kind: "misconfigured" | "unreachable" | "invalid";
  message: string;
}

export type GuestResult = GuestSuccess | GuestFailure;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isGuestSession(value: unknown): value is GuestSession {
  return (
    isRecord(value) &&
    typeof value.token === "string" &&
    value.role === "guest" &&
    typeof value.sandbox_id === "string" &&
    typeof value.expires_at === "string"
  );
}

/**
 * Starts a login-free guest session (`POST /v1/guest`). Never throws.
 *
 * The returned `token` is sent as `Authorization: Bearer <token>` on later calls (projects,
 * versions, builds). Judges need no account; this is the first step of the judge flow.
 */
export async function startGuestSession(): Promise<GuestResult> {
  const config = getApiConfig();
  if (!config.ok) {
    return { kind: "misconfigured", message: config.reason };
  }

  let response: Response;
  try {
    response = await fetch(`${config.baseUrl}/v1/guest`, {
      method: "POST",
      cache: "no-store",
      signal: AbortSignal.timeout(TIMEOUT_MS),
    });
  } catch (error) {
    const name = error instanceof Error ? error.name : "Error";
    return { kind: "unreachable", message: `The API did not answer (${name}).` };
  }

  if (!response.ok) {
    return { kind: "unreachable", message: `The API answered HTTP ${response.status}.` };
  }

  let body: unknown;
  try {
    body = await response.json();
  } catch {
    return { kind: "invalid", message: "The API answered with a body that is not JSON." };
  }
  if (!isGuestSession(body)) {
    return { kind: "invalid", message: "The API answered with an unexpected shape." };
  }
  return { kind: "ok", session: body };
}
