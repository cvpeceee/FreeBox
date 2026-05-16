import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { Plus, HardDrive, Database } from 'lucide-react';
import { storage } from '@/lib/api';
import { Spinner } from '@/components/ui/Spinner';
import { formatDate } from '@/lib/utils';

export default function StoragePage() {
  const qc = useQueryClient();
  const [newBucket, setNewBucket] = useState('');
  const [createError, setCreateError] = useState<string | null>(null);

  const datasourcesQ = useQuery({
    queryKey: ['datasources'],
    queryFn: storage.datasources,
  });

  const bucketsQ = useQuery({
    queryKey: ['buckets'],
    queryFn: storage.buckets,
  });

  const createMut = useMutation({
    mutationFn: (name: string) => storage.createBucket(name),
    onSuccess: () => {
      setNewBucket('');
      setCreateError(null);
      qc.invalidateQueries({ queryKey: ['buckets'] });
    },
    onError: (err) => setCreateError((err as Error).message),
  });

  return (
    <div className="h-full bg-[#1b1b1b] text-[#e5e5e5] p-6 space-y-6 max-w-3xl">
      <h1 className="text-xl font-semibold text-[#e5e5e5]">Storage</h1>

      {/* Storage usage (placeholder) */}
      <div className="rounded-lg border border-[#2d2d2d] bg-[#252525] p-5 space-y-3">
        <div className="flex items-center justify-between">
          <div>
            <p className="text-2xl font-bold text-[#e5e5e5]">0.7 <span className="text-lg font-normal text-[#9ca3af]">GB</span></p>
            <p className="text-xs text-[#9ca3af] mt-0.5">used of 5 GB</p>
          </div>
          <div className="h-12 w-12 rounded-full border-4 border-[#0078d4] flex items-center justify-center">
            <span className="text-xs font-bold text-[#0078d4]">14%</span>
          </div>
        </div>
        <div className="h-2 bg-[#333] rounded-full overflow-hidden">
          <div className="h-full w-[14%] bg-[#0078d4] rounded-full" />
        </div>
        <p className="text-xs text-[#666]">4.3 GB available</p>
      </div>

      {/* Data sources */}
      <div className="rounded-lg border border-[#2d2d2d] bg-[#252525] overflow-hidden">
        <div className="flex items-center gap-2 px-4 py-3 border-b border-[#2d2d2d]">
          <Database className="h-4 w-4 text-[#0078d4]" />
          <h2 className="font-medium text-[#e5e5e5] text-sm">Data sources</h2>
        </div>
        <div className="p-4">
          {datasourcesQ.isLoading ? (
            <Spinner />
          ) : (
            <ul className="divide-y divide-[#2d2d2d]">
              {datasourcesQ.data?.map((ds) => (
                <li key={ds.name} className="py-3 flex items-center justify-between">
                  <div>
                    <p className="text-sm font-medium text-[#e5e5e5]">{ds.name}</p>
                    <p className="text-xs text-[#9ca3af] capitalize">{ds.provider}</p>
                  </div>
                  {ds.region && (
                    <span className="text-xs bg-[#2d2d2d] text-[#9ca3af] border border-[#3d3d3d] rounded px-2 py-0.5">
                      {ds.region}
                    </span>
                  )}
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>

      {/* Buckets */}
      <div className="rounded-lg border border-[#2d2d2d] bg-[#252525] overflow-hidden">
        <div className="flex items-center gap-2 px-4 py-3 border-b border-[#2d2d2d]">
          <HardDrive className="h-4 w-4 text-[#0078d4]" />
          <h2 className="font-medium text-[#e5e5e5] text-sm">Buckets</h2>
        </div>
        <div className="p-4">
          <div className="flex gap-2 mb-4">
            <input
              type="text"
              placeholder="New bucket name"
              value={newBucket}
              onChange={(e) => setNewBucket(e.target.value)}
              className="flex-1 bg-[#1b1b1b] border border-[#3d3d3d] rounded px-3 py-1.5 text-sm text-[#e5e5e5] placeholder-[#666] outline-none focus:border-[#0078d4] transition-colors"
            />
            <button
              disabled={createMut.isPending || !newBucket.trim()}
              onClick={() => createMut.mutate(newBucket.trim())}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded bg-[#0078d4] text-white text-sm font-medium hover:bg-[#106ebe] transition-colors disabled:opacity-40"
            >
              <Plus className="h-4 w-4" />
              Create
            </button>
          </div>
          {createError && <p className="text-xs text-red-400 mb-3">{createError}</p>}

          {bucketsQ.isLoading ? (
            <Spinner />
          ) : bucketsQ.data?.buckets.length === 0 ? (
            <p className="text-sm text-[#9ca3af]">No buckets found.</p>
          ) : (
            <ul className="divide-y divide-[#2d2d2d]">
              {bucketsQ.data?.buckets.map((b) => (
                <li key={b.name} className="py-3 flex items-center justify-between">
                  <p className="text-sm font-medium text-[#e5e5e5]">{b.name}</p>
                  {b.creation_date && (
                    <span className="text-xs text-[#666]">{formatDate(b.creation_date)}</span>
                  )}
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </div>
  );
}
