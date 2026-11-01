import type { ErrorBody, SessionInfo } from './generated';

export class ApiError extends Error {
  status: number;
  field: string | null;

  constructor(status: number, message: string, field: string | null = null) {
    super(message);
    this.status = status;
    this.field = field;
  }
}

let csrf = '';

export function setCsrf(token: string) {
  csrf = token;
}

type Method = 'GET' | 'POST' | 'PUT' | 'DELETE';

async function request<T>(method: Method, path: string, body?: unknown, raw?: Blob | ArrayBuffer, extra: Record<string, string> = {}): Promise<T> {
  const headers: Record<string, string> = { ...extra };
  if (method !== 'GET') headers['x-csrf-token'] = csrf;
  let payload: BodyInit | undefined;
  if (raw !== undefined) {
    headers['content-type'] = 'application/octet-stream';
    payload = raw;
  } else if (body !== undefined) {
    headers['content-type'] = 'application/json';
    payload = JSON.stringify(body);
  }
  let response: Response;
  try {
    response = await fetch(`/api${path}`, { method, headers, body: payload, credentials: 'same-origin' });
  } catch {
    throw new ApiError(0, 'The panel does not answer.');
  }
  if (response.status === 401 && !path.startsWith('/login') && !path.startsWith('/setup')) {
    if (!location.pathname.startsWith('/login') && !location.pathname.startsWith('/setup')) {
      location.assign(`/login?next=${encodeURIComponent(location.pathname)}`);
    }
  }
  if (!response.ok) {
    let error: ErrorBody | null = null;
    try {
      error = (await response.json()) as ErrorBody;
    } catch {
      error = null;
    }
    throw new ApiError(response.status, error?.error ?? `The request failed (${response.status}).`, error?.field ?? null);
  }
  if (response.status === 204 || response.status === 202) return undefined as T;
  const type = response.headers.get('content-type') ?? '';
  if (type.includes('application/json')) return (await response.json()) as T;
  return (await response.blob()) as T;
}

export const api = {
  get: <T>(path: string) => request<T>('GET', path),
  post: <T>(path: string, body: unknown = {}) => request<T>('POST', path, body),
  put: <T>(path: string, body: unknown) => request<T>('PUT', path, body),
  del: <T>(path: string, body?: unknown) => request<T>('DELETE', path, body),
  upload: <T>(path: string, data: Blob | ArrayBuffer, headers: Record<string, string> = {}) => request<T>('POST', path, undefined, data, headers)
};

export function base64Text(text: string): string {
  let binary = '';
  for (const byte of new TextEncoder().encode(text)) binary += String.fromCharCode(byte);
  return btoa(binary);
}

export async function loadSession(): Promise<SessionInfo | null> {
  try {
    const session = await api.get<SessionInfo>('/session');
    setCsrf(session.csrf);
    return session;
  } catch (error) {
    if (error instanceof ApiError && error.status === 401) return null;
    throw error;
  }
}

export function fieldErrors(error: unknown): { form: string | null; fields: Record<string, string> } {
  if (error instanceof ApiError) {
    if (error.field) return { form: null, fields: { [error.field]: error.message } };
    return { form: error.message, fields: {} };
  }
  return { form: 'Something went wrong. Try again.', fields: {} };
}
