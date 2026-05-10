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

// Attach Bearer token to every request.
http.interceptors.request.use((config) => {
  const token = getAccessToken();
  if (token) {
    config.headers.Authorization = `Bearer ${token}`;
  }
  return config;
});

// On 401, attempt a token refresh and retry once.
let refreshing: Promise<void> | null = null;

// Auth routes must never trigger the refresh loop.
const AUTH_ROUTES = ['/api/v1/auth/login', '/api/v1/auth/register', '/api/v1/auth/refresh'];

http.interceptors.response.use(
  (res) => res,
  async (err: AxiosError<ApiError>) => {
    const original = err.config as typeof err.config & { _retry?: boolean };
    const url = original?.url ?? '';
    const isAuthRoute = AUTH_ROUTES.some((r) => url.includes(r));

    if (err.response?.status === 401 && !original._retry && !isAuthRoute) {
      original._retry = true;
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
        })().finally(() => {
          refreshing = null;
        });
      }
      await refreshing;
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
