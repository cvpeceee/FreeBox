import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { Unlink, Shield, Activity } from 'lucide-react';
import { auth } from '@/lib/api';
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
    <div className="h-full bg-[#1b1b1b] text-[#e5e5e5] p-6 space-y-6 max-w-3xl">
      <h1 className="text-xl font-semibold text-[#e5e5e5]">Settings</h1>

      {/* Linked OAuth providers */}
      <div className="rounded-lg border border-[#2d2d2d] bg-[#252525] overflow-hidden">
        <div className="flex items-center gap-2 px-4 py-3 border-b border-[#2d2d2d]">
          <Shield className="h-4 w-4 text-[#0078d4]" />
          <h2 className="font-medium text-[#e5e5e5] text-sm">Linked sign-in providers</h2>
        </div>
        <div className="p-4">
          {providersQ.isLoading ? (
            <Spinner />
          ) : providersQ.data?.length === 0 ? (
            <p className="text-sm text-[#9ca3af]">No OAuth providers linked.</p>
          ) : (
            <ul className="divide-y divide-[#2d2d2d]">
              {providersQ.data?.map((p) => (
                <li key={p.provider} className="flex items-center justify-between py-3">
                  <div>
                    <p className="text-sm font-medium text-[#e5e5e5] capitalize">{p.provider}</p>
                    <p className="text-xs text-[#9ca3af]">{p.provider_email ?? p.provider_username}</p>
                  </div>
                  <button
                    disabled={unlinkMut.isPending}
                    onClick={() => unlinkMut.mutate(p.provider)}
                    className="flex items-center gap-1.5 px-3 py-1.5 rounded text-xs text-red-400 hover:bg-[#2d2d2d] border border-transparent hover:border-[#3d3d3d] transition-colors disabled:opacity-40"
                  >
                    <Unlink className="h-3.5 w-3.5" />
                    Unlink
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>

      {/* Audit log */}
      <div className="rounded-lg border border-[#2d2d2d] bg-[#252525] overflow-hidden">
        <div className="flex items-center gap-2 px-4 py-3 border-b border-[#2d2d2d]">
          <Activity className="h-4 w-4 text-[#0078d4]" />
          <h2 className="font-medium text-[#e5e5e5] text-sm">Account activity</h2>
        </div>
        <div className="p-4">
          {auditQ.isLoading ? (
            <Spinner />
          ) : auditQ.data?.events.length === 0 ? (
            <p className="text-sm text-[#9ca3af]">No activity recorded yet.</p>
          ) : (
            <ul className="divide-y divide-[#2d2d2d]">
              {auditQ.data?.events.map((ev) => (
                <li key={ev.id} className="py-3 flex items-start justify-between gap-4">
                  <div>
                    <p className="text-sm font-medium text-[#e5e5e5]">{ev.event_type}</p>
                    <p className="text-xs text-[#9ca3af]">
                      via {ev.source}
                      {ev.provider ? ` / ${ev.provider}` : ''}
                    </p>
                  </div>
                  <time className="text-xs text-[#666] shrink-0">{formatDate(ev.created_at)}</time>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </div>
  );
}
