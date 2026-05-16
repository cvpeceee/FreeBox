// ---------------------------------------------------------------------------
// Types that mirror the server's JSON response shapes.
// Keep in sync with apps/server/src/api/files.rs, auth.rs, oauth.rs, storage.rs
// ---------------------------------------------------------------------------

export interface AuthTokens {
  access_token: string;
  refresh_token: string;
  expires_in: number;
}

export interface FileMeta {
  file_id: string;
  encrypted_name: string;
  size_bytes: number;
  total_chunks: number;
  encrypted_key_envelope: string;
  content_hash: string;
  created_at: string;
}

export interface FileListResponse {
  files: FileMeta[];
  total: number;
  limit: number;
  offset: number;
}

export interface TrashFile {
  file_id: string;
  encrypted_name: string;
  size_bytes: number;
  total_chunks: number;
  content_hash: string;
  created_at: string;
  deleted_at: string;
}

export interface TrashListResponse {
  files: TrashFile[];
  total: number;
  limit: number;
  offset: number;
}

export interface UploadInitRequest {
  total_chunks: number;
  size_bytes: number;
  encrypted_key_envelope: string;
  content_hash: string;
  encrypted_name: string;
}

export interface UploadInitResponse {
  upload_id: string;
  chunk_size: number;
}

export interface UploadCompleteResponse {
  file_id: string;
}

export interface OAuthProvider {
  provider: string;
  provider_email: string | null;
  provider_username: string | null;
  avatar_url: string | null;
  created_at: string;
}

export interface AuditEvent {
  id: string;
  user_id: string;
  event_type: string;
  source: string;
  provider: string | null;
  provider_user_id: string | null;
  details: Record<string, unknown>;
  created_at: string;
}

export interface AuditEventsResponse {
  events: AuditEvent[];
  total: number;
  limit: number;
  offset: number;
}

export interface Datasource {
  name: string;
  provider: string;
  region: string | null;
}

export interface Bucket {
  name: string;
  creation_date: string | null;
}

export interface BucketListResponse {
  buckets: Bucket[];
}

export interface ApiError {
  error: string;
  message: string;
}
