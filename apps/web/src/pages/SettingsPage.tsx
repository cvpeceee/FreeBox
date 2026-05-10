import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Unlink } from 'lucide-react';
import { auth } from '@/lib/api';
import { Button } from '@/components/ui/Button';
import { Card, CardHeader, CardBody } from '@/components/ui/Card';
import { Spinner } from '@/components/ui/Spinner';
import { formatDate } from '@/lib/utils';

export default function SettingsPage() {
  const qc = useQueryClient();

  const providersQ = useQuery({
    queryKey: ['providers'],
    queryFn: auth.listProviders,
  });

  const auditQ = useQuery({
    queryKey: ['audit-events'],
    queryFn: () => auth.auditEvents({ limit: 20 }),
  });

  const unlinkMut = useMutation({
    mutationFn: (provider: string) => auth.unlinkProvider(provider),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['providers'] }),
  });

  return (
    <div className="p-6 space-y-6 max-w-3xl">
      <h1 className="text-xl font-semibold text-gray-900">Settings</h1>

      {/* Linked OAuth providers */}
      <Card>
        <CardHeader>
          <h2 className="font-medium text-gray-900">Linked sign-in providers</h2>
        </CardHeader>
        <CardBody>
          {providersQ.isLoading ? (
            <Spinner />
          ) : providersQ.data?.length === 0 ? (
            <p className="text-sm text-gray-500">No OAuth providers linked.</p>
          ) : (
            <ul className="divide-y divide-gray-100">
              {providersQ.data?.map((p) => (
                <li key={p.provider} className="flex items-center justify-between py-3">
                  <div>
                    <p className="text-sm font-medium text-gray-800 capitalize">{p.provider}</p>
                    <p className="text-xs text-gray-500">{p.provider_email ?? p.provider_username}</p>
                  </div>
                  <Button
                    variant="ghost"
                    size="sm"
                    loading={unlinkMut.isPending}
                    onClick={() => unlinkMut.mutate(p.provider)}
                  >
                    <Unlink className="h-4 w-4 text-red-500" />
                    Unlink
                  </Button>
                </li>
              ))}
            </ul>
          )}
        </CardBody>
      </Card>

      {/* Audit log */}
      <Card>
        <CardHeader>
          <h2 className="font-medium text-gray-900">Account activity</h2>
        </CardHeader>
        <CardBody>
          {auditQ.isLoading ? (
            <Spinner />
          ) : auditQ.data?.events.length === 0 ? (
            <p className="text-sm text-gray-500">No activity recorded yet.</p>
          ) : (
            <ul className="divide-y divide-gray-100">
              {auditQ.data?.events.map((ev) => (
                <li key={ev.id} className="py-3 flex items-start justify-between gap-4">
                  <div>
                    <p className="text-sm font-medium text-gray-800">{ev.event_type}</p>
                    <p className="text-xs text-gray-500">
                      via {ev.source}
                      {ev.provider ? ` / ${ev.provider}` : ''}
                    </p>
                  </div>
                  <time className="text-xs text-gray-400 shrink-0">{formatDate(ev.created_at)}</time>
                </li>
              ))}
            </ul>
          )}
        </CardBody>
      </Card>
    </div>
  );
}
