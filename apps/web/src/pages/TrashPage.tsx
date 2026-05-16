import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { RotateCcw, RefreshCw, Trash2, FileText } from 'lucide-react';
import { files } from '@/lib/api';
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
    <div className="flex flex-col h-full bg-[#1b1b1b] text-[#e5e5e5] p-6 space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-xl font-semibold text-[#e5e5e5]">Recycle bin</h1>
          <p className="text-sm text-[#9ca3af] mt-0.5">
            Files deleted within the last 30 days. After that they are permanently removed.
          </p>
        </div>
        <button
          onClick={() => refetch()}
          className="p-2 rounded hover:bg-[#2d2d2d] text-[#9ca3af] hover:text-white transition-colors"
          title="Refresh"
        >
          <RefreshCw className="h-4 w-4" />
        </button>
      </div>

      {isLoading ? (
        <div className="flex justify-center py-20">
          <Spinner />
        </div>
      ) : isError ? (
        <p className="text-red-400 text-sm">Failed to load recycle bin.</p>
      ) : data?.files.length === 0 ? (
        <div className="text-center py-20 text-[#555]">
          <Trash2 className="h-12 w-12 mx-auto mb-3 opacity-30" />
          <p className="text-sm">Recycle bin is empty.</p>
        </div>
      ) : (
        <>
          <div className="grid grid-cols-[1fr_180px_140px_80px] text-xs text-[#666] border-b border-[#2d2d2d] py-2 px-2 select-none">
            <span>Name</span>
            <span>Deleted</span>
            <span>File size</span>
            <span />
          </div>

          <div className="divide-y divide-[#242424]">
            {data?.files.map((file) => (
              <div
                key={file.file_id}
                className="grid grid-cols-[1fr_180px_140px_80px] items-center px-2 py-2.5 hover:bg-[#252525] rounded transition-colors group"
              >
                <div className="flex items-center gap-3 min-w-0">
                  <div className="w-8 h-8 rounded bg-[#3d3d3d] flex items-center justify-center shrink-0">
                    <FileText className="h-4 w-4 text-[#9ca3af]" />
                  </div>
                  <span className="text-sm text-[#e5e5e5] truncate font-mono text-xs">
                    {file.encrypted_name}
                  </span>
                </div>
                <span className="text-xs text-[#9ca3af]">{formatDate(file.deleted_at)}</span>
                <span className="text-xs text-[#9ca3af]">{formatBytes(file.size_bytes)}</span>
                <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity justify-end">
                  <button
                    onClick={() => restoreMut.mutate(file.file_id)}
                    disabled={restoreMut.isPending}
                    className="p-1.5 rounded hover:bg-[#333] text-[#9ca3af] hover:text-[#0078d4] transition-colors disabled:opacity-40"
                    title="Restore"
                  >
                    <RotateCcw className="h-3.5 w-3.5" />
                  </button>
                </div>
              </div>
            ))}
          </div>

          {data && data.total > PAGE_SIZE && (
            <div className="flex items-center justify-between text-sm text-[#9ca3af] py-4">
              <span>
                {offset + 1}–{Math.min(offset + PAGE_SIZE, data.total)} of {data.total}
              </span>
              <div className="flex gap-2">
                <button
                  disabled={offset === 0}
                  onClick={() => setOffset(Math.max(0, offset - PAGE_SIZE))}
                  className="px-3 py-1.5 rounded bg-[#2d2d2d] text-sm disabled:opacity-40 hover:bg-[#333] transition-colors"
                >
                  Previous
                </button>
                <button
                  disabled={offset + PAGE_SIZE >= data.total}
                  onClick={() => setOffset(offset + PAGE_SIZE)}
                  className="px-3 py-1.5 rounded bg-[#2d2d2d] text-sm disabled:opacity-40 hover:bg-[#333] transition-colors"
                >
                  Next
                </button>
              </div>
            </div>
          )}
        </>
      )}
    </div>
  );
}
