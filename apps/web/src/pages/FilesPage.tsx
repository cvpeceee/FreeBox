import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Trash2, Download, RefreshCw, FileText, X, Plus } from 'lucide-react';
import { clsx } from 'clsx';
import { files } from '@/lib/api';
import type { FileMeta } from '@/lib/api';
import { downloadAndDecrypt } from '@/lib/crypto';
import { Spinner } from '@/components/ui/Spinner';
import { formatBytes, formatDate } from '@/lib/utils';
import UploadModal from './UploadModal';

const PAGE_SIZE = 50;
const FILTER_TABS = ['All', 'Documents', 'Images', 'Videos', 'Other'];

function FileIcon() {
  return (
    <div className="w-8 h-8 rounded bg-[#106ebe] flex items-center justify-center shrink-0">
      <FileText className="h-4 w-4 text-white" />
    </div>
  );
}

export default function FilesPage() {
  const qc = useQueryClient();
  const [offset, setOffset] = useState(0);
  const [showUpload, setShowUpload] = useState(false);
  const [downloadingId, setDownloadingId] = useState<string | null>(null);
  const [activeFilter, setActiveFilter] = useState('All');
  const [showBanner, setShowBanner] = useState(true);

  const { data, isLoading, isError, refetch } = useQuery({
    queryKey: ['files', offset],
    queryFn: () => files.list({ limit: PAGE_SIZE, offset }),
  });

  const deleteMut = useMutation({
    mutationFn: (fileId: string) => files.delete(fileId),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['files'] }),
  });

  async function handleDownload(file: FileMeta) {
    setDownloadingId(file.file_id);
    try {
      await downloadAndDecrypt(file);
    } catch (err) {
      console.error('Download failed:', err);
    } finally {
      setDownloadingId(null);
    }
  }

  return (
    <div className="flex flex-col h-full bg-[#1b1b1b] text-[#e5e5e5]">
      {/* Promo banner */}
      {showBanner && (
        <div className="flex items-center gap-3 px-6 py-3 bg-[#0f3a5e] border-b border-[#1a4f7a] shrink-0">
          <span className="text-2xl leading-none">☁</span>
          <div className="flex-1 min-w-0">
            <span className="font-semibold text-sm text-white">Get 100 GB free for a month</span>
            <span className="text-sm text-[#a8d4f0] ml-2">
              Start your trial now to get more storage for all your files and photos.
            </span>
          </div>
          <button className="shrink-0 text-xs font-semibold bg-white text-[#0f3a5e] rounded px-3 py-1.5 hover:bg-gray-100 transition-colors whitespace-nowrap">
            Start free trial
          </button>
          <button
            onClick={() => setShowBanner(false)}
            className="shrink-0 p-1 text-[#a8d4f0] hover:text-white transition-colors"
            title="Dismiss"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
      )}

      {/* Toolbar */}
      <div className="flex items-center gap-2 px-6 py-3 border-b border-[#2d2d2d] shrink-0">
        <button
          onClick={() => setShowUpload(true)}
          className="flex items-center gap-2 bg-[#2d2d2d] hover:bg-[#333] border border-[#3d3d3d] text-[#e5e5e5] rounded px-3 py-1.5 text-sm font-medium transition-colors"
        >
          <Plus className="h-4 w-4" />
          Create or upload
        </button>
        <button
          onClick={() => refetch()}
          className="p-2 rounded hover:bg-[#2d2d2d] text-[#9ca3af] hover:text-white transition-colors"
          title="Refresh"
        >
          <RefreshCw className="h-4 w-4" />
        </button>
      </div>

      {/* Recent label + filter tabs */}
      <div className="flex items-center gap-2 px-6 pt-4 pb-2 shrink-0">
        <span className="text-sm font-medium text-[#e5e5e5] mr-1">Recent</span>
        <div className="flex items-center gap-1">
          {FILTER_TABS.map((tab) => (
            <button
              key={tab}
              onClick={() => setActiveFilter(tab)}
              className={clsx(
                'px-3 py-1 rounded-full text-sm font-medium transition-colors border',
                activeFilter === tab
                  ? 'bg-[#2d2d2d] text-white border-[#555]'
                  : 'text-[#9ca3af] hover:bg-[#252525] hover:text-white border-transparent',
              )}
            >
              {tab}
            </button>
          ))}
        </div>
        <div className="ml-auto">
          <input
            type="text"
            placeholder="Filter by name or person"
            className="bg-transparent border border-[#3d3d3d] rounded px-3 py-1 text-xs text-[#9ca3af] placeholder-[#555] outline-none focus:border-[#555] w-48 transition-colors"
          />
        </div>
      </div>

      {/* File list */}
      <div className="flex-1 overflow-auto px-6">
        {isLoading ? (
          <div className="flex justify-center py-20">
            <Spinner />
          </div>
        ) : isError ? (
          <p className="text-red-400 text-sm py-8">Failed to load files.</p>
        ) : data?.files.length === 0 ? (
          <div className="text-center py-20 text-[#555]">
            <FileText className="h-12 w-12 mx-auto mb-3 opacity-30" />
            <p className="text-sm">No files yet. Upload your first file to get started.</p>
          </div>
        ) : (
          <>
            {/* Column headers */}
            <div className="grid grid-cols-[1fr_180px_140px_72px] text-xs text-[#666] border-b border-[#2d2d2d] py-2 px-2 select-none">
              <span>Name</span>
              <span>Opened</span>
              <span>File size</span>
              <span />
            </div>

            {/* File rows */}
            <div className="divide-y divide-[#242424]">
              {data?.files.map((file) => (
                <div
                  key={file.file_id}
                  className="grid grid-cols-[1fr_180px_140px_72px] items-center px-2 py-2.5 hover:bg-[#252525] rounded transition-colors group"
                >
                  <div className="flex items-center gap-3 min-w-0">
                    <FileIcon />
                    <span className="text-sm text-[#e5e5e5] truncate font-mono text-xs">
                      {file.encrypted_name}
                    </span>
                  </div>
                  <span className="text-xs text-[#9ca3af]">{formatDate(file.created_at)}</span>
                  <span className="text-xs text-[#9ca3af]">{formatBytes(file.size_bytes)}</span>
                  <div className="flex items-center gap-1 opacity-0 group-hover:opacity-100 transition-opacity justify-end">
                    <button
                      onClick={() => handleDownload(file)}
                      disabled={downloadingId !== null}
                      className="p-1.5 rounded hover:bg-[#333] text-[#9ca3af] hover:text-white transition-colors disabled:opacity-40"
                      title="Download"
                    >
                      <Download className="h-3.5 w-3.5" />
                    </button>
                    <button
                      onClick={() => deleteMut.mutate(file.file_id)}
                      className="p-1.5 rounded hover:bg-[#333] text-[#9ca3af] hover:text-red-400 transition-colors"
                      title="Delete"
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </button>
                  </div>
                </div>
              ))}
            </div>

            {/* Pagination */}
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

      {showUpload && <UploadModal onClose={() => setShowUpload(false)} />}
    </div>
  );
}
