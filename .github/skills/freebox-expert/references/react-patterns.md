# FreeBox React Patterns Deep-Dive

This reference teaches React, TypeScript, and frontend architecture using FreeBox's web app as examples.

---

## Technology Stack

| Library | Role | Why |
|---------|------|-----|
| React 18 | UI rendering | Concurrent features, Suspense |
| TypeScript | Type safety | Catches API contract mismatches at compile time |
| React Router v6 | Routing | Nested routes, outlet-based layouts |
| TanStack Query (React Query) | **Server state** | Caching, background refetch, optimistic updates |
| Zustand | **Client state** | Auth status, UI preferences |
| Vite | Build tool | Fast HMR, native ESM |
| Tailwind CSS | Styling | Utility-first, no CSS file proliferation |
| Lucide React | Icons | Tree-shakeable SVG icons |

---

## 1. State Architecture: Two-Store Pattern

FreeBox separates state into two distinct concerns:

```
┌─────────────────────────────────────────────────────────────┐
│                    State Management                          │
│                                                             │
│  ┌─────────────────────┐    ┌──────────────────────────┐   │
│  │    Zustand Store     │    │    React Query Cache     │   │
│  │  (CLIENT state)      │    │  (SERVER state)          │   │
│  │                      │    │                          │   │
│  │  - isAuthenticated   │    │  - files list            │   │
│  │  - UI preferences    │    │  - file metadata         │   │
│  │  - local UI state    │    │  - storage buckets       │   │
│  │                      │    │  - audit events          │   │
│  └─────────────────────┘    └──────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

**Rule of thumb**: If the data lives on the server, use React Query. If it's local UI state that doesn't need to be fetched, use Zustand.

---

## 2. Zustand — Auth Store

**File**: `apps/web/src/stores/authStore.ts`

```typescript
export const useAuthStore = create<AuthState>((set) => ({
  isAuthenticated: !!getAccessToken(),        // initialized from localStorage

  setAuthenticated: (value) => set({ isAuthenticated: value }),

  logout: () => {
    clearTokens();                             // removes tokens from localStorage
    set({ isAuthenticated: false });
  },
}));
```

### Key Concepts

**`create<AuthState>`**: Zustand's `create` function returns a hook. The generic `<AuthState>` tells TypeScript what shape the store has — any access of `state.nonExistentField` is a compile error.

**No Provider needed**: Unlike Redux or React Context, Zustand stores are module-level singletons. Any component can import `useAuthStore` directly — no wrapping `<Provider>`.

**Selector pattern**: Always select only what you need:
```typescript
// GOOD — component only re-renders when isAuthenticated changes
const isAuthenticated = useAuthStore((s) => s.isAuthenticated);

// BAD — component re-renders on ANY store change
const store = useAuthStore();
```

### Design Question: `localStorage` for Tokens
The auth store reads from `localStorage` on init (`!!getAccessToken()`). This is a common pattern but has security implications:
- **XSS vulnerability**: Any script on the page can read `localStorage`. If the app has an XSS vulnerability, tokens can be stolen.
- **Better alternative**: `HttpOnly` cookies (unreadable by JavaScript). Trade-off: requires CORS + SameSite configuration.

FreeBox's current approach is pragmatic for a self-hosted app but should be flagged for production hardening.

---

## 3. React Query — Server State

**File**: `apps/web/src/pages/FilesPage.tsx`

### `useQuery` — Fetching Data
```typescript
const { data, isLoading, isError, refetch } = useQuery({
  queryKey: ['files', offset],              // cache key — unique per page
  queryFn: () => files.list({ limit: PAGE_SIZE, offset }),
});
```

**`queryKey`**: The cache identifier. React Query re-fetches when the key changes. `['files', offset]` means "the files at this offset" — navigating to a different page automatically triggers a fetch.

**Automatic re-fetching**: After 30 seconds (`staleTime: 30_000` in `App.tsx`), the cache entry is considered stale and re-fetched in the background when the component mounts.

### `useMutation` — Modifying Data
```typescript
const deleteMut = useMutation({
  mutationFn: (fileId: string) => files.delete(fileId),
  onSuccess: () => qc.invalidateQueries({ queryKey: ['files'] }),
});
```

**`invalidateQueries`**: After a successful delete, all `['files', *]` cache entries are marked stale and re-fetched. This is the **invalidation pattern** — the simplest way to keep the cache in sync after mutations.

**Alternative — Optimistic Updates**: For a snappier UX, remove the file from the local cache immediately, then roll back if the API call fails:
```typescript
onMutate: async (fileId) => {
  await qc.cancelQueries({ queryKey: ['files'] });
  const previous = qc.getQueryData(['files', offset]);
  qc.setQueryData(['files', offset], (old) => ({
    ...old,
    files: old.files.filter(f => f.file_id !== fileId),
  }));
  return { previous };
},
onError: (_, __, context) => {
  qc.setQueryData(['files', offset], context.previous); // rollback
},
```

---

## 4. Protected Routes — Auth-Guarded Navigation

**File**: `apps/web/src/components/ProtectedRoute.tsx` + `apps/web/src/App.tsx`

### Route Structure
```tsx
// App.tsx
<Routes>
  <Route path="/login" element={<LoginPage />} />          {/* public */}
  <Route path="/register" element={<RegisterPage />} />    {/* public */}

  <Route element={<ProtectedRoute />}>                     {/* auth guard */}
    <Route element={<AppLayout />}>                        {/* shared layout */}
      <Route index element={<FilesPage />} />
      <Route path="trash" element={<TrashPage />} />
    </Route>
  </Route>
</Routes>
```

### How `ProtectedRoute` Works
```tsx
export function ProtectedRoute() {
  const isAuthenticated = useAuthStore((s) => s.isAuthenticated);
  if (!isAuthenticated) return <Navigate to="/login" replace />;
  return <Outlet />;   // renders the matched child route
}
```

`<Outlet />` is React Router v6's way of rendering child routes in a parent component. The parent (`ProtectedRoute`) acts as a wrapper — it either redirects or renders the children.

**`replace` on Navigate**: Uses `history.replace` instead of `history.push` — the `/login` page won't be added to the browser history, so pressing Back doesn't loop the user back to the protected route.

### Nested Layouts
```tsx
<Route element={<ProtectedRoute />}>
  <Route element={<AppLayout />}>   {/* sidebar + header wrapper */}
    <Route index element={<FilesPage />} />
  </Route>
</Route>
```

Two levels of `<Outlet />`:
1. `ProtectedRoute` renders `<Outlet />` → which renders `AppLayout`
2. `AppLayout` renders `<Outlet />` → which renders `FilesPage`

This is how you share layout (sidebar, nav) across protected pages without duplicating JSX.

---

## 5. Download Flow — Blob URL Pattern

**File**: `apps/web/src/pages/FilesPage.tsx` → `handleDownload`

```typescript
async function handleDownload(file: FileMeta) {
  const buffer = await files.downloadChunk(file.file_id, 0);
  const blob = new Blob([buffer], { type: 'application/octet-stream' });
  const url = URL.createObjectURL(blob);          // temporary in-memory URL
  const a = document.createElement('a');
  a.href = url;
  a.download = file.file_id;
  a.click();
  URL.revokeObjectURL(url);                       // release memory immediately
}
```

### Design Flaw: Missing Client-Side Decryption
Currently, the code downloads the raw (still-encrypted) chunk and triggers a browser download of ciphertext. The comment notes this:
> "In a full E2EE impl this would go through the crypto Worker to decrypt."

For true E2EE, the download flow should be:
1. Fetch chunk(s) from server (gets encrypted bytes)
2. Fetch sealed FileKey from server
3. Decrypt FileKey using Signal session key (via Web Crypto API)
4. Decrypt chunk bytes using AES-256-GCM + FileKey (via Web Crypto API or WASM)
5. Create Blob from plaintext bytes → trigger download

The crypto should run in a **Web Worker** to avoid blocking the main thread.

### Design Flaw: Missing `a.remove()` after click
The dynamically created `<a>` element is clicked but never removed from the DOM. Add `document.body.appendChild(a)` before `a.click()` and `a.remove()` after for cross-browser compatibility (some browsers require the element to be in the DOM for the click to trigger a download).

---

## 6. TypeScript API Contract Typing

**File**: `apps/web/src/lib/api.ts`

Strong typing of API responses ensures the frontend catches contract mismatches at compile time:
```typescript
export interface FileMeta {
  file_id: string;
  file_name: string;   // encrypted, base64-encoded
  file_size: number;
  chunk_count: number;
  created_at: string;  // ISO 8601
}
```

When the server adds a new field or changes a type, TypeScript will surface the mismatch during `tsc` build — not at runtime in production.

**Teaching point**: Use `zod` for runtime validation of API responses — TypeScript types are erased at runtime, so an unexpected API shape won't throw a TypeScript error, it'll just silently be `undefined`.

---

## Design Flaws Checklist — React/Frontend

- [ ] **Token storage in `localStorage`**: Vulnerable to XSS. Consider `HttpOnly` cookies for production.
- [ ] **Missing client-side decryption**: Download flow currently delivers ciphertext to the user. Full E2EE requires a crypto Web Worker.
- [ ] **File name displayed as encrypted bytes**: `FilesPage.tsx` shows `file_name` but it's encrypted — the column header says "(encrypted)". Need client-side decryption of file names.
- [ ] **Missing `a.remove()` in download handler**: Cross-browser download may silently fail in some browsers.
- [ ] **No error boundaries**: If `FilesPage` throws, the whole app crashes. Wrap page-level components in `<ErrorBoundary>`.
- [ ] **Pagination not prefetched**: React Query can prefetch the next page while the user reads the current one (`queryClient.prefetchQuery`).
- [ ] **No upload progress**: `UploadModal.tsx` should show chunk-level upload progress using `onUploadProgress` in Axios.
