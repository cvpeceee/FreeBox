/**
 * UploadModal — chunked file upload with per-file AES-256-GCM encryption.
 *
 * Each file is split into 4 MiB chunks. A fresh random AES-256-GCM key is
 * generated per file, and every chunk is encrypted before being sent to the
 * server. The server never sees plaintext — only the JSON-serialised
 * ChunkCiphertext produced by prepareUpload().
 */

import { useState, useRef } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import { X, Upload } from 'lucide-react';
import { files } from '@/lib/api';
import { prepareUpload } from '@/lib/crypto';
import { Button } from '@/components/ui/Button';

const CHUNK_SIZE = 4 * 1024 * 1024; // 4 MiB
const PARALLELISM = 8;

interface UploadItem {
  file: File;
  progress: number; // 0–100
  status: 'pending' | 'uploading' | 'done' | 'error';
  error?: string;
}

interface UploadModalProps {
  onClose: () => void;
}

export default function UploadModal({ onClose }: UploadModalProps) {
  const qc = useQueryClient();
  const inputRef = useRef<HTMLInputElement>(null);
  const [items, setItems] = useState<UploadItem[]>([]);
  const [uploading, setUploading] = useState(false);

  function setItemField(index: number, patch: Partial<UploadItem>) {
    setItems((prev) => prev.map((it, i) => (i === index ? { ...it, ...patch } : it)));
  }

  function handleFileSelect(e: React.ChangeEvent<HTMLInputElement>) {
    const selected = Array.from(e.target.files ?? []);
    setItems(selected.map((file) => ({ file, progress: 0, status: 'pending' })));
  }

  async function uploadFile(item: UploadItem, index: number): Promise<void> {
    const { file } = item;
    const totalChunks = Math.max(1, Math.ceil(file.size / CHUNK_SIZE));
    setItemField(index, { status: 'uploading', progress: 0 });

    try {
      // Generate a fresh AES-256-GCM key, compute content hash, and get encrypt helper.
      const ctx = await prepareUpload(file);

      const { upload_id } = await files.uploadInit({
        total_chunks: totalChunks,
        size_bytes: file.size,
        encrypted_key_envelope: ctx.encrypted_key_envelope,
        content_hash: ctx.content_hash,
        encrypted_name: ctx.encrypted_name,
      });

      // Upload chunks with bounded parallelism.
      let chunkIndex = 0;
      let completed = 0;

      async function runNext(): Promise<void> {
        while (chunkIndex < totalChunks) {
          const current = chunkIndex++;
          const start = current * CHUNK_SIZE;
          const end = Math.min(start + CHUNK_SIZE, file.size);
          const plaintext = new Uint8Array(await file.slice(start, end).arrayBuffer()) as Uint8Array<ArrayBuffer>;
          const ciphertext = await ctx.encryptChunk(current, plaintext);

          // Retry up to 3 times on transient server errors (e.g. 502 while
          // the Rust server is still warming up).
          let lastErr: unknown;
          for (let attempt = 0; attempt < 3; attempt++) {
            try {
              await files.uploadChunk(upload_id, current, ciphertext);
              lastErr = undefined;
              break;
            } catch (err) {
              lastErr = err;
              await new Promise((r) => setTimeout(r, 500 * (attempt + 1)));
            }
          }
          if (lastErr) throw lastErr;

          completed++;
          setItemField(index, { progress: Math.round((completed / totalChunks) * 90) });
        }
      }

      const workers = Array.from({ length: Math.min(PARALLELISM, totalChunks) }, runNext);
      await Promise.all(workers);

      await files.uploadComplete(upload_id);
      setItemField(index, { status: 'done', progress: 100 });
    } catch (err) {
      setItemField(index, { status: 'error', error: (err as Error).message });
    }
  }

  async function handleUpload() {
    setUploading(true);
    await Promise.all(items.map((item, i) => uploadFile(item, i)));
    setUploading(false);
    qc.invalidateQueries({ queryKey: ['files'] });
  }

  const allDone = items.length > 0 && items.every((it) => it.status === 'done');

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4">
      <div className="w-full max-w-lg bg-white rounded-xl shadow-xl">
        {/* Header */}
        <div className="flex items-center justify-between px-6 py-4 border-b border-gray-200">
          <h2 className="font-semibold text-gray-900">Upload Files</h2>
          <button onClick={onClose} className="text-gray-400 hover:text-gray-600">
            <X className="h-5 w-5" />
          </button>
        </div>

        <div className="px-6 py-4 space-y-4">
          {/* Drop zone */}
          <div
            className="border-2 border-dashed border-gray-300 rounded-lg p-8 text-center cursor-pointer hover:border-indigo-400 transition-colors"
            onClick={() => inputRef.current?.click()}
          >
            <Upload className="h-8 w-8 mx-auto text-gray-400 mb-2" />
            <p className="text-sm text-gray-600">Click to select files or drop them here</p>
            <input
              ref={inputRef}
              type="file"
              multiple
              className="hidden"
              onChange={handleFileSelect}
            />
          </div>

          {/* File list with progress */}
          {items.length > 0 && (
            <ul className="space-y-2 max-h-60 overflow-y-auto">
              {items.map((item, i) => (
                <li key={i} className="space-y-1">
                  <div className="flex items-center justify-between text-sm">
                    <span className="truncate max-w-xs text-gray-700">{item.file.name}</span>
                    <span
                      className={
                        item.status === 'done'
                          ? 'text-green-600'
                          : item.status === 'error'
                            ? 'text-red-600'
                            : 'text-gray-500'
                      }
                    >
                      {item.status === 'done'
                        ? 'Done'
                        : item.status === 'error'
                          ? 'Error'
                          : `${item.progress}%`}
                    </span>
                  </div>
                  <div className="h-1.5 bg-gray-100 rounded-full overflow-hidden">
                    <div
                      className={`h-full rounded-full transition-all ${
                        item.status === 'error' ? 'bg-red-500' : 'bg-indigo-600'
                      }`}
                      style={{ width: `${item.progress}%` }}
                    />
                  </div>
                  {item.error && <p className="text-xs text-red-600">{item.error}</p>}
                </li>
              ))}
            </ul>
          )}
        </div>

        {/* Footer */}
        <div className="flex justify-end gap-2 px-6 py-4 border-t border-gray-200">
          <Button variant="secondary" onClick={onClose}>
            {allDone ? 'Close' : 'Cancel'}
          </Button>
          {!allDone && (
            <Button
              onClick={handleUpload}
              loading={uploading}
              disabled={items.length === 0 || uploading}
            >
              Upload {items.length > 0 ? `${items.length} file${items.length > 1 ? 's' : ''}` : ''}
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}
