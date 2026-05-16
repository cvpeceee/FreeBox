import { NavLink, Outlet, useNavigate } from 'react-router-dom';
import { Home, Files, Trash2, HardDrive, Settings, Cloud, Search } from 'lucide-react';
import { clsx } from 'clsx';
import { auth } from '@/lib/api';
import { useAuthStore } from '@/stores/authStore';

const sidebarNav = [
  { to: '/', label: 'Home', icon: Home, end: true },
  { to: '/files', label: 'My files', icon: Files, end: false },
  { to: '/trash', label: 'Recycle bin', icon: Trash2, end: false },
];

export default function AppLayout() {
  const navigate = useNavigate();
  const logout = useAuthStore((s) => s.logout);

  async function handleLogout() {
    try {
      await auth.logout();
    } catch {
      // Best-effort: clear tokens even if the server request fails.
    }
    logout();
    navigate('/login');
  }

  return (
    <div className="flex flex-col h-screen bg-[#1b1b1b] text-[#e5e5e5]">
      {/* ── Top navbar ── */}
      <header className="h-12 flex items-center gap-3 px-4 bg-[#1b1b1b] border-b border-[#2d2d2d] shrink-0">
        {/* Logo + tab pills */}
        <div className="flex items-center gap-1">
          <Cloud className="h-6 w-6 text-[#0078d4] mr-1" />
          {(['Photos', 'Files'] as const).map((tab) => (
            <button
              key={tab}
              className={clsx(
                'px-3 py-1 text-sm rounded font-medium transition-colors',
                tab === 'Files'
                  ? 'text-white'
                  : 'text-[#9ca3af] hover:text-white hover:bg-[#252525]',
              )}
            >
              {tab}
            </button>
          ))}
        </div>

        {/* Search */}
        <div className="flex-1 max-w-lg">
          <div className="flex items-center gap-2 bg-[#2d2d2d] border border-[#3d3d3d] rounded-full px-4 py-1.5">
            <Search className="h-4 w-4 text-[#9ca3af] shrink-0" />
            <input
              type="text"
              placeholder="Search"
              className="flex-1 bg-transparent text-sm text-[#e5e5e5] placeholder-[#9ca3af] outline-none"
            />
          </div>
        </div>

        {/* Right actions */}
        <div className="ml-auto flex items-center gap-2">
          <button
            onClick={() => navigate('/storage')}
            className="hidden sm:flex items-center gap-1.5 text-xs border border-[#0078d4] text-[#0078d4] rounded-sm px-3 py-1.5 hover:bg-[#0078d4]/10 transition-colors font-medium"
          >
            Get more storage
          </button>
          <button
            onClick={() => navigate('/settings')}
            className="p-2 rounded hover:bg-[#2d2d2d] text-[#9ca3af] hover:text-white transition-colors"
            title="Settings"
          >
            <Settings className="h-4 w-4" />
          </button>
          <button
            onClick={handleLogout}
            className="h-8 w-8 rounded-full bg-[#0078d4] flex items-center justify-center text-white text-xs font-bold hover:bg-[#106ebe] transition-colors"
            title="Sign out"
          >
            FB
          </button>
        </div>
      </header>

      <div className="flex flex-1 overflow-hidden">
        {/* ── Sidebar ── */}
        <aside className="w-52 shrink-0 flex flex-col bg-[#1b1b1b] border-r border-[#2d2d2d] overflow-y-auto">
          {/* Username placeholder */}
          <div className="px-4 pt-4 pb-2 text-sm font-semibold text-[#e5e5e5]">
            FreeBox user
          </div>

          {/* Primary navigation */}
          <nav className="px-2 space-y-0.5">
            {sidebarNav.map(({ to, label, icon: Icon, end }) => (
              <NavLink
                key={to}
                to={to}
                end={end}
                className={({ isActive }) =>
                  clsx(
                    'flex items-center gap-3 px-3 py-2 rounded text-sm transition-colors',
                    isActive
                      ? 'bg-[#2d2d2d] text-white'
                      : 'text-[#c7c7c7] hover:bg-[#252525] hover:text-white',
                  )
                }
              >
                <Icon className="h-4 w-4 shrink-0" />
                {label}
              </NavLink>
            ))}
          </nav>

          {/* Browse by section */}
          <div className="mt-4 px-2">
            <p className="px-3 pb-1 text-[10px] uppercase tracking-wider text-[#555] font-semibold">
              Browse files by
            </p>
            <NavLink
              to="/storage"
              className={({ isActive }) =>
                clsx(
                  'flex items-center gap-3 px-3 py-2 rounded text-sm transition-colors',
                  isActive
                    ? 'bg-[#2d2d2d] text-white'
                    : 'text-[#c7c7c7] hover:bg-[#252525] hover:text-white',
                )
              }
            >
              <HardDrive className="h-4 w-4 shrink-0" />
              Storage
            </NavLink>
          </div>

          {/* Storage widget */}
          <div className="mt-auto p-3 space-y-3">
            <div className="rounded-lg bg-[#252525] p-3 text-xs space-y-2">
              <p className="text-[#c7c7c7] leading-snug">
                Get storage for all your files and photos.
              </p>
              <button
                onClick={() => navigate('/storage')}
                className="w-full flex items-center justify-center gap-1.5 bg-[#2d2d2d] border border-[#3d3d3d] text-[#c7c7c7] hover:text-white rounded px-3 py-1.5 font-medium transition-colors"
              >
                <HardDrive className="h-3.5 w-3.5" />
                Buy storage
              </button>
            </div>
            <div className="px-1">
              <div className="flex justify-between text-[11px] mb-1">
                <span className="text-[#0078d4] font-semibold">0.7 GB</span>
                <span className="text-[#555]">used of 5 GB (14%)</span>
              </div>
              <div className="h-1 bg-[#333] rounded-full overflow-hidden">
                <div className="h-full w-[14%] bg-[#0078d4] rounded-full" />
              </div>
            </div>
          </div>
        </aside>

        {/* ── Main content ── */}
        <main className="flex-1 overflow-auto bg-[#1b1b1b]">
          <Outlet />
        </main>
      </div>
    </div>
  );
}
