/** Runtime configuration read from public environment variables. */

const DEV_DEFAULT_API_URL = "http://localhost:8000";

export type ApiConfig =
  | { ok: true; baseUrl: string }
  | { ok: false; reason: string };

/**
 * Resolves the Core API base URL from `NEXT_PUBLIC_API_URL`.
 * In production a missing or invalid value is an error, never a silent default.
 */
export function getApiConfig(): ApiConfig {
  const raw = process.env.NEXT_PUBLIC_API_URL?.trim();

  if (!raw) {
    if (process.env.NODE_ENV === "production") {
      return { ok: false, reason: "NEXT_PUBLIC_API_URL is not set." };
    }
    return { ok: true, baseUrl: DEV_DEFAULT_API_URL };
  }

  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    return { ok: false, reason: "NEXT_PUBLIC_API_URL is not a valid URL." };
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    return { ok: false, reason: "NEXT_PUBLIC_API_URL must use http or https." };
  }
  return { ok: true, baseUrl: url.origin };
}
