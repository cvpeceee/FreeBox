import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { RotateCcw, RefreshCw } from 'lucide-react';
import { files } from '@/lib/api';
import { Button } from '@/components/ui/Button';
import { Spinner } from '@/components/ui/Spinner';
import { formatBytes, formatDate } from '@/lib/utils';

const PAGE_SIZE = 50;

export default function TrashPage() {
  const qc = useQueryClient();
  const [offset, setOffset] = useState(0);

  const { data, isLoading, isError, refetch } = useQuery({
    queryKey: ['trash', offset],
    queryFn: () => files.trash({ limit: PAGE_SIZE, offset }),
  });

  const restoreMut = useMutation({
    mutationFn: (fileId: string) => files.restore(fileId),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ['trash'] });
      qc.invalidateQueries({ queryKey: ['files'] });
    },
  });

  return (
    <div className="p-6 space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold text-gray-900">Trash</h1>
          <p className="text-sm text-gray-500 mt-0.5">
            Files deleted within the last 30 days. After that they are permanently removed.
          </p>
        </div>
        <Button variant="ghost" size="sm" onClick={() => refetch()}>
          <RefreshCw className="h-4 w-4" />
        </Button>
      </div>

      {isLoading ? (
        <div className="flex justify-center py-20">
          <Spinner />
        </div>
      ) : isError ? (
        <p className="text-red-600 text-sm">Failed to load trash.</p>
      ) : data?.files.length === 0 ? (
        <div className="text-center py-20 text-gray-500">
          <p className="text-sm">Trash is empty.</p>
        </div>
      ) : (
        <>
          <div className="rounded-lg border border-gray-200 bg-white overflow-hidden">
            <table className="min-w-full divide-y divide-gray-200 text-sm">
              <thead className="bg-gray-50">
                <tr>
                  <th className="px-4 py-3 text-left font-medium text-gray-500">Name (encrypted)</th>
                  <th className="px-4 py-3 text-right font-medium text-gray-500">Size</th>
                  <th className="px-4 py-3 text-right font-medium text-gray-500">Deleted</th>
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
                    <td className="px-4 py-3 text-right text-gray-500 text-xs">
                      {formatDate(file.deleted_at)}
                    </td>
                    <td className="px-4 py-3 text-right">
                      <Button
                        variant="ghost"
                        size="sm"
                        title="Restore file"
                        loading={restoreMut.isPending}
                        onClick={() => restoreMut.mutate(file.file_id)}
                      >
                        <RotateCcw className="h-4 w-4 text-indigo-600" />
                        Restore
                      </Button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>

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
    </div>
  );
}
