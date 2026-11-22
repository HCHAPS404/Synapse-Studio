import { getApiConfig } from "./config";

const TIMEOUT_MS = 8000;

/**
 * Shape of `GET /health` in `services/core`.
 * Temporary hand-written type: replace it with types generated from the OpenAPI schema.
 */
export interface Health {
  status: "ok" | "degraded";
  version: string;
  environment: string;
  database: {
    status: "ok" | "unconfigured" | "error";
    latency_ms: number | null;
    detail: string | null;
  };
}

export interface HealthAnswer {
  kind: "ok" | "degraded";
  baseUrl: string;
  data: Health;
}

export interface HealthFailure {
  kind: "misconfigured" | "unreachable" | "invalid";
  message: string;
}

export type HealthResult = HealthAnswer | HealthFailure;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function isHealth(value: unknown): value is Health {
  if (!isRecord(value) || !isRecord(value.database)) return false;
  return (
    (value.status === "ok" || value.status === "degraded") &&
    typeof value.version === "string" &&
    typeof value.environment === "string" &&
    (value.database.status === "ok" ||
      value.database.status === "unconfigured" ||
      value.database.status === "error")
  );
}

/** Calls `GET /health` on the Core API. Never throws. */
export async function fetchHealth(): Promise<HealthResult> {
  const config = getApiConfig();
  if (!config.ok) {
    return { kind: "misconfigured", message: config.reason };
  }

  let response: Response;
  try {
    response = await fetch(`${config.baseUrl}/health`, {
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
  if (!isHealth(body)) {
    return { kind: "invalid", message: "The API answered with an unexpected shape." };
  }
  return { kind: body.status, baseUrl: config.baseUrl, data: body };
}
