import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Trash2, Download, Upload, RefreshCw } from 'lucide-react';
import { files } from '@/lib/api';
import type { FileMeta } from '@/lib/api';
import { Button } from '@/components/ui/Button';
import { Spinner } from '@/components/ui/Spinner';
import { formatBytes, formatDate } from '@/lib/utils';
import UploadModal from './UploadModal';

const PAGE_SIZE = 50;

export default function FilesPage() {
  const qc = useQueryClient();
  const [offset, setOffset] = useState(0);
  const [showUpload, setShowUpload] = useState(false);

  const { data, isLoading, isError, refetch } = useQuery({
    queryKey: ['files', offset],
    queryFn: () => files.list({ limit: PAGE_SIZE, offset }),
  });

  const deleteMut = useMutation({
    mutationFn: (fileId: string) => files.delete(fileId),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['files'] }),
  });

  function handleDownload(file: FileMeta) {
    // Direct chunk download — in a full E2EE impl this goes through the
    // crypto Worker to decrypt. For now, download chunk 0 as a demo.
    const url = `/api/v1/files/${file.file_id}/chunk/0`;
    const a = document.createElement('a');
    a.href = url;
    a.download = file.file_id;
    a.click();
  }

  return (
    <div className="p-6 space-y-4">
      {/* Header */}
      <div className="flex items-center justify-between">
        <h1 className="text-xl font-semibold text-gray-900">My Files</h1>
        <div className="flex items-center gap-2">
          <Button variant="ghost" size="sm" onClick={() => refetch()}>
            <RefreshCw className="h-4 w-4" />
          </Button>
          <Button size="sm" onClick={() => setShowUpload(true)}>
            <Upload className="h-4 w-4" />
            Upload
          </Button>
        </div>
      </div>

      {/* Table */}
      {isLoading ? (
        <div className="flex justify-center py-20">
          <Spinner />
        </div>
      ) : isError ? (
        <p className="text-red-600 text-sm">Failed to load files.</p>
      ) : data?.files.length === 0 ? (
        <div className="text-center py-20 text-gray-500">
          <p className="text-sm">No files yet. Upload your first file to get started.</p>
        </div>
      ) : (
        <>
          <div className="rounded-lg border border-gray-200 bg-white overflow-hidden">
            <table className="min-w-full divide-y divide-gray-200 text-sm">
              <thead className="bg-gray-50">
                <tr>
                  <th className="px-4 py-3 text-left font-medium text-gray-500">Name (encrypted)</th>
                  <th className="px-4 py-3 text-right font-medium text-gray-500">Size</th>
                  <th className="px-4 py-3 text-right font-medium text-gray-500">Chunks</th>
                  <th className="px-4 py-3 text-right font-medium text-gray-500">Created</th>
                  <th className="px-4 py-3" />
                </tr>
              </thead>
              <tbody className="divide-y divide-gray-100">
                {data?.files.map((file) => (
                  <tr key={file.file_id} className="hover:bg-gray-50">
                    <td className="px-4 py-3 font-mono text-xs text-gray-600 max-w-xs truncate">
                      {file.encrypted_name}
                    </td>
                    <td className="px-4 py-3 text-right text-gray-700">
                      {formatBytes(file.size_bytes)}
                    </td>
                    <td className="px-4 py-3 text-right text-gray-700">{file.total_chunks}</td>
                    <td className="px-4 py-3 text-right text-gray-500 text-xs">
                      {formatDate(file.created_at)}
                    </td>
                    <td className="px-4 py-3 text-right">
                      <div className="flex items-center justify-end gap-1">
                        <Button
                          variant="ghost"
                          size="sm"
                          title="Download"
                          onClick={() => handleDownload(file)}
                        >
                          <Download className="h-4 w-4" />
                        </Button>
                        <Button
                          variant="ghost"
                          size="sm"
                          title="Move to trash"
                          loading={deleteMut.isPending}
                          onClick={() => deleteMut.mutate(file.file_id)}
                        >
                          <Trash2 className="h-4 w-4 text-red-500" />
                        </Button>
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

          {/* Pagination */}
          {data && data.total > PAGE_SIZE && (
            <div className="flex items-center justify-between text-sm text-gray-600">
              <span>
                {offset + 1}–{Math.min(offset + PAGE_SIZE, data.total)} of {data.total}
              </span>
              <div className="flex gap-2">
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={offset === 0}
                  onClick={() => setOffset(Math.max(0, offset - PAGE_SIZE))}
                >
                  Previous
                </Button>
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={offset + PAGE_SIZE >= data.total}
                  onClick={() => setOffset(offset + PAGE_SIZE)}
                >
                  Next
                </Button>
              </div>
            </div>
          )}
        </>
      )}

      {showUpload && <UploadModal onClose={() => setShowUpload(false)} />}
    </div>
  );
}
