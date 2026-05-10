import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import { Plus } from 'lucide-react';
import { storage } from '@/lib/api';
import { Button } from '@/components/ui/Button';
import { Card, CardHeader, CardBody } from '@/components/ui/Card';
import { Input } from '@/components/ui/Input';
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
    <div className="p-6 space-y-6 max-w-3xl">
      <h1 className="text-xl font-semibold text-gray-900">Storage</h1>

      {/* Data sources */}
      <Card>
        <CardHeader>
          <h2 className="font-medium text-gray-900">Data sources</h2>
        </CardHeader>
        <CardBody>
          {datasourcesQ.isLoading ? (
            <Spinner />
          ) : (
            <ul className="divide-y divide-gray-100">
              {datasourcesQ.data?.map((ds) => (
                <li key={ds.id} className="py-3 flex items-center justify-between">
                  <div>
                    <p className="text-sm font-medium text-gray-800">{ds.name}</p>
                    <p className="text-xs text-gray-500 capitalize">{ds.provider}</p>
                  </div>
                  {ds.region && (
                    <span className="text-xs bg-gray-100 text-gray-600 rounded px-2 py-0.5">
                      {ds.region}
                    </span>
                  )}
                </li>
              ))}
            </ul>
          )}
        </CardBody>
      </Card>

      {/* Buckets */}
      <Card>
        <CardHeader>
          <h2 className="font-medium text-gray-900">Buckets</h2>
        </CardHeader>
        <CardBody>
          <div className="flex gap-2 mb-4">
            <Input
              placeholder="New bucket name"
              value={newBucket}
              onChange={(e) => setNewBucket(e.target.value)}
              className="flex-1"
              error={createError ?? undefined}
            />
            <Button
              size="sm"
              loading={createMut.isPending}
              disabled={!newBucket.trim()}
              onClick={() => createMut.mutate(newBucket.trim())}
            >
              <Plus className="h-4 w-4" />
              Create
            </Button>
          </div>

          {bucketsQ.isLoading ? (
            <Spinner />
          ) : bucketsQ.data?.buckets.length === 0 ? (
            <p className="text-sm text-gray-500">No buckets found.</p>
          ) : (
            <ul className="divide-y divide-gray-100">
              {bucketsQ.data?.buckets.map((b) => (
                <li key={b.name} className="py-3 flex items-center justify-between">
                  <p className="text-sm font-medium text-gray-800">{b.name}</p>
                  {b.creation_date && (
                    <span className="text-xs text-gray-400">{formatDate(b.creation_date)}</span>
                  )}
                </li>
              ))}
            </ul>
          )}
        </CardBody>
      </Card>
    </div>
  );
}
