/**
 * Typed API client for the FreeBox server.
 *
 * All methods:
 * - Automatically attach the Bearer token from localStorage
 * - Automatically refresh the token on 401 and retry once
 * - Throw a typed ApiError on non-2xx responses
 */

import axios, { type AxiosInstance, AxiosError } from 'axios';

// ---------------------------------------------------------------------------
// Client-side password hashing (PBKDF2 via Web Crypto API)
// ---------------------------------------------------------------------------

/**
 * Derives a deterministic hex string from password + salt using PBKDF2-SHA-256.
 *
 * The raw password NEVER leaves the browser — only this derived value is sent
 * over the network. The server re-hashes it with Argon2id (double hashing).
 *
 * PBKDF2 is used here because Argon2 is not available in the native Web Crypto
 * API. This will be upgraded to Argon2id via the WASM crypto module.
 */
export async function hashPassword(password: string, salt: string): Promise<string> {
  const enc = new TextEncoder();
  const keyMaterial = await crypto.subtle.importKey(
    'raw',
    enc.encode(password),
    'PBKDF2',
    false,
    ['deriveBits'],
  );
  const bits = await crypto.subtle.deriveBits(
    {
      name: 'PBKDF2',
      salt: enc.encode(salt),
      iterations: 600_000,
      hash: 'SHA-256',
    },
    keyMaterial,
    256,
  );
  return Array.from(new Uint8Array(bits))
    .map((b) => b.toString(16).padStart(2, '0'))
    .join('');
}
import type {
  AuthTokens,
  FileListResponse,
  FileMeta,
  TrashListResponse,
  UploadInitRequest,
  UploadInitResponse,
  UploadCompleteResponse,
  OAuthProvider,
  AuditEventsResponse,
  Datasource,
  BucketListResponse,
  ApiError,
} from './types';

// Keys for localStorage
const ACCESS_TOKEN_KEY = 'fbx_access_token';
const REFRESH_TOKEN_KEY = 'fbx_refresh_token';

export function getAccessToken(): string | null {
  return localStorage.getItem(ACCESS_TOKEN_KEY);
}

export function getRefreshToken(): string | null {
  return localStorage.getItem(REFRESH_TOKEN_KEY);
}

export function saveTokens(tokens: AuthTokens): void {
  localStorage.setItem(ACCESS_TOKEN_KEY, tokens.access_token);
  localStorage.setItem(REFRESH_TOKEN_KEY, tokens.refresh_token);
}

export function clearTokens(): void {
  localStorage.removeItem(ACCESS_TOKEN_KEY);
  localStorage.removeItem(REFRESH_TOKEN_KEY);
}

// ---------------------------------------------------------------------------
// Axios instance
// ---------------------------------------------------------------------------

const http: AxiosInstance = axios.create({
  baseURL: '/',
  headers: { 'Content-Type': 'application/json' },
});

// Auth routes must never trigger the refresh loop.
const AUTH_ROUTES = ['/api/v1/auth/login', '/api/v1/auth/register', '/api/v1/auth/refresh'];

// ---------------------------------------------------------------------------
// Shared refresh guard — ensures only one refresh call is in-flight at a time
// regardless of how many requests trigger it concurrently.
// ---------------------------------------------------------------------------

let refreshing: Promise<void> | null = null;
let refreshFailedAt: number | null = null;
const REFRESH_CIRCUIT_BREAK_MS = 10_000; // don't retry refresh for 10 s after a failure

function doRefresh(): Promise<void> {
  // Circuit breaker: if refresh just failed (e.g. server down) don't hammer it.
  if (refreshFailedAt !== null && Date.now() - refreshFailedAt < REFRESH_CIRCUIT_BREAK_MS) {
    return Promise.reject(new Error('Server unreachable — retry in a moment.'));
  }
  if (!refreshing) {
    refreshing = (async () => {
      const refreshToken = getRefreshToken();
      if (!refreshToken) {
        clearTokens();
        throw new Error('Session expired. Please log in again.');
      }
      const res = await axios.post<AuthTokens>('/api/v1/auth/refresh', {
        refresh_token: refreshToken,
      });
      saveTokens(res.data);
      refreshFailedAt = null; // clear on success
    })().finally(() => {
      refreshing = null;
    });
    refreshing.catch(() => {
      refreshFailedAt = Date.now();
    });
  }
  return refreshing;
}

/** Decode the JWT `exp` claim without a library. Returns null if unparseable. */
function jwtExp(token: string): number | null {
  try {
    const payload = JSON.parse(atob(token.split('.')[1]));
    return typeof payload.exp === 'number' ? payload.exp : null;
  } catch {
    return null;
  }
}

// ---------------------------------------------------------------------------
// Request interceptor — proactively refresh the token if it is expired or
// within 60 s of expiry so API calls never hit a 401 on normal page reloads.
// ---------------------------------------------------------------------------

http.interceptors.request.use(async (config) => {
  const isAuthRoute = AUTH_ROUTES.some((r) => (config.url ?? '').includes(r));
  if (!isAuthRoute) {
    const token = getAccessToken();
    if (token) {
      const exp = jwtExp(token);
      if (exp !== null && exp * 1000 < Date.now() + 60_000) {
        // Token is expired or expiring in <60 s — refresh before sending.
        try {
          await doRefresh();
        } catch {
          // Refresh failed; let the request go out and the response
          // interceptor will handle the resulting 401.
        }
      }
    }
  }
  const freshToken = getAccessToken();
  if (freshToken) {
    config.headers.Authorization = `Bearer ${freshToken}`;
  }
  return config;
});

// ---------------------------------------------------------------------------
// Response interceptor — defensive fallback for unexpected 401s.
// ---------------------------------------------------------------------------

http.interceptors.response.use(
  (res) => res,
  async (err: AxiosError<ApiError>) => {
    const original = err.config as typeof err.config & { _retry?: boolean };
    const url = original?.url ?? '';
    const isAuthRoute = AUTH_ROUTES.some((r) => url.includes(r));

    if (err.response?.status === 401 && !original._retry && !isAuthRoute) {
      original._retry = true;
      try {
        await doRefresh();
      } catch {
        clearTokens();
        throw new Error('Session expired. Please log in again.');
      }
      return http(original);
    }

    // Re-throw as a plain error with the server message for UI display.
    if (!err.response) {
      throw new Error('Cannot reach the server. Is the backend running?');
    }
    const message =
      err.response.data?.message ??
      (err.response.status === 401 ? 'Invalid username or password.' : err.message);
    throw new Error(message);
  },
);

// ---------------------------------------------------------------------------
// Auth
// ---------------------------------------------------------------------------

export const auth = {
  async register(body: {
    username: string;
    email: string;
    password_hash: string;
    argon2_salt: string;
    prekey_bundle: unknown;
  }): Promise<AuthTokens> {
    const res = await http.post<AuthTokens>('/api/v1/auth/register', body);
    saveTokens(res.data);
    return res.data;
  },

  async getSalt(username: string): Promise<string> {
    const res = await http.get<{ username: string; argon2_salt: string }>(
      `/api/v1/auth/salt/${encodeURIComponent(username)}`,
    );
    return res.data.argon2_salt;
  },

  async login(body: { username: string; password_hash: string }): Promise<AuthTokens> {
    const res = await http.post<AuthTokens>('/api/v1/auth/login', body);
    saveTokens(res.data);
    return res.data;
  },

  async logout(): Promise<void> {
    const refreshToken = getRefreshToken();
    await http.post('/api/v1/auth/logout', { refresh_token: refreshToken });
    clearTokens();
  },

  oauthUrl(provider: string): string {
    return `/api/v1/auth/oauth/${provider}`;
  },

  async configuredProviders(): Promise<string[]> {
    const res = await http.get<{ providers: string[] }>('/api/v1/auth/oauth/configured');
    return res.data.providers;
  },

  async listProviders(): Promise<OAuthProvider[]> {
    const res = await http.get<{ providers: OAuthProvider[] }>('/api/v1/auth/providers');
    return res.data.providers;
  },

  async unlinkProvider(provider: string): Promise<void> {
    await http.delete(`/api/v1/auth/oauth/${provider}/unlink`);
  },

  async auditEvents(params?: {
    limit?: number;
    offset?: number;
  }): Promise<AuditEventsResponse> {
    const res = await http.get<AuditEventsResponse>('/api/v1/auth/audit-events', { params });
    return res.data;
  },
};

// ---------------------------------------------------------------------------
// Files
// ---------------------------------------------------------------------------

export const files = {
  async list(params?: { limit?: number; offset?: number }): Promise<FileListResponse> {
    const res = await http.get<FileListResponse>('/api/v1/files', { params });
    return res.data;
  },

  async getMeta(fileId: string): Promise<FileMeta> {
    const res = await http.get<FileMeta>(`/api/v1/files/${fileId}`);
    return res.data;
  },

  async delete(fileId: string): Promise<void> {
    await http.delete(`/api/v1/files/${fileId}`);
  },

  async restore(fileId: string): Promise<void> {
    await http.post(`/api/v1/files/${fileId}/restore`);
  },

  async rename(fileId: string, encryptedName: string): Promise<void> {
    await http.patch(`/api/v1/files/${fileId}`, { encrypted_name: encryptedName });
  },

  async trash(params?: { limit?: number; offset?: number }): Promise<TrashListResponse> {
    const res = await http.get<TrashListResponse>('/api/v1/files/trash', { params });
    return res.data;
  },

  async uploadInit(body: UploadInitRequest): Promise<UploadInitResponse> {
    const res = await http.post<UploadInitResponse>('/api/v1/files/upload/init', body);
    return res.data;
  },

  async uploadChunk(
    uploadId: string,
    chunkIndex: number,
    data: Uint8Array,
    onProgress?: (pct: number) => void,
  ): Promise<void> {
    await http.put(`/api/v1/files/upload/${uploadId}`, data, {
      headers: {
        'Content-Type': 'application/octet-stream',
        'X-Chunk-Index': String(chunkIndex),
      },
      onUploadProgress: (e) => {
        if (onProgress && e.total) {
          onProgress(Math.round((e.loaded / e.total) * 100));
        }
      },
    });
  },

  async uploadComplete(uploadId: string): Promise<UploadCompleteResponse> {
    const res = await http.post<UploadCompleteResponse>(
      `/api/v1/files/upload/${uploadId}/complete`,
    );
    return res.data;
  },

  downloadChunkUrl(fileId: string, chunkIndex: number): string {
    return `/api/v1/files/${fileId}/chunk/${chunkIndex}`;
  },

  async downloadChunk(fileId: string, chunkIndex: number): Promise<ArrayBuffer> {
    const res = await http.get<ArrayBuffer>(
      `/api/v1/files/${fileId}/chunk/${chunkIndex}`,
      { responseType: 'arraybuffer' },
    );
    return res.data;
  },
};

// ---------------------------------------------------------------------------
// Storage
// ---------------------------------------------------------------------------

export const storage = {
  async datasources(): Promise<Datasource[]> {
    const res = await http.get<{ datasources: Datasource[] }>('/api/v1/storage/datasources');
    return res.data.datasources;
  },

  async buckets(): Promise<BucketListResponse> {
    const res = await http.get<BucketListResponse>('/api/v1/storage/buckets');
    return res.data;
  },

  async createBucket(name: string): Promise<void> {
    await http.post('/api/v1/storage/buckets', { name });
  },
};
