// Browser-side helper for calling this app's own JSON API routes.

export interface ApiResult<T> {
  ok: boolean;
  status: number;
  data?: T;
  error?: string;
  code?: string;
}

export async function api<T = unknown>(path: string, method = "POST", body?: unknown): Promise<ApiResult<T>> {
  try {
    const res = await fetch(path, {
      method,
      headers: { "content-type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
      credentials: "same-origin",
    });
    const json = (await res.json().catch(() => ({}))) as Record<string, unknown>;
    if (!res.ok) {
      return { ok: false, status: res.status, error: String(json.message ?? "Request failed."), code: String(json.error ?? "") };
    }
    return { ok: true, status: res.status, data: json as T };
  } catch {
    return { ok: false, status: 0, error: "Network error. Check your connection and try again." };
  }
}
