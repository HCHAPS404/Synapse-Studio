// A thin, typed client over the SYNAPSE Core API.
//
// The types come from ./schema.ts, which is GENERATED from spec/api/openapi.json and must
// never be hand-edited (CI fails on drift). This file is the only hand-written part: a
// small fetch wrapper that carries the guest bearer token and parses the shared ErrorBody.

import type { ErrorBody, GuestSessionOut } from "./schema";

export * from "./schema";

export interface ClientOptions {
  /** Base URL of the Core API, e.g. "https://synapse-core.example.com". */
  baseUrl: string;
  /** Optional bearer token (a guest token from POST /v1/guest, or a user/service token). */
  token?: string;
  /** Injectable fetch, for tests and non-browser runtimes. */
  fetch?: typeof fetch;
}

/** Thrown when the API returns a non-2xx response; carries the parsed ErrorBody when present. */
export class ApiError extends Error {
  constructor(
    readonly status: number,
    readonly body: ErrorBody | null,
  ) {
    super(body?.message ?? `HTTP ${status}`);
    this.name = "ApiError";
  }

  /** The NExxxx code, when the server returned one. */
  get code(): string | undefined {
    return this.body?.code;
  }
}

export class SynapseClient {
  private readonly baseUrl: string;
  private token?: string;
  private readonly doFetch: typeof fetch;

  constructor(options: ClientOptions) {
    this.baseUrl = options.baseUrl.replace(/\/+$/, "");
    this.token = options.token;
    this.doFetch = options.fetch ?? fetch;
  }

  /** Starts a guest session and stores its token on this client. */
  async startGuestSession(): Promise<GuestSessionOut> {
    const session = await this.request<GuestSessionOut>("POST", "/v1/guest");
    this.token = session.token;
    return session;
  }

  /** A typed request. `path` must already be a concrete path (no templating here). */
  async request<T>(method: string, path: string, body?: unknown): Promise<T> {
    const headers: Record<string, string> = { Accept: "application/json" };
    if (this.token) headers.Authorization = `Bearer ${this.token}`;
    if (body !== undefined) headers["Content-Type"] = "application/json";
    const response = await this.doFetch(`${this.baseUrl}${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    if (!response.ok) {
      let parsed: ErrorBody | null = null;
      try {
        parsed = (await response.json()) as ErrorBody;
      } catch {
        parsed = null;
      }
      throw new ApiError(response.status, parsed);
    }
    if (response.status === 204) return undefined as T;
    return (await response.json()) as T;
  }
}
