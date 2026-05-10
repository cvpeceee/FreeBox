import { NavLink, Outlet, useNavigate } from 'react-router-dom';
import { Files, Trash2, Settings, LogOut, HardDrive } from 'lucide-react';
import { clsx } from 'clsx';
import { auth } from '@/lib/api';
import { useAuthStore } from '@/stores/authStore';

const nav = [
  { to: '/', label: 'Files', icon: Files, end: true },
  { to: '/trash', label: 'Trash', icon: Trash2, end: false },
  { to: '/storage', label: 'Storage', icon: HardDrive, end: false },
  { to: '/settings', label: 'Settings', icon: Settings, end: false },
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
    <div className="flex h-screen bg-gray-50">
      {/* Sidebar */}
      <aside className="w-56 shrink-0 flex flex-col bg-white border-r border-gray-200">
        {/* Logo */}
        <div className="h-16 flex items-center px-5 border-b border-gray-200">
          <span className="text-xl font-bold text-gray-900">FreeBox</span>
        </div>

        {/* Navigation */}
        <nav className="flex-1 py-4 px-3 space-y-1">
          {nav.map(({ to, label, icon: Icon, end }) => (
            <NavLink
              key={to}
              to={to}
              end={end}
              className={({ isActive }) =>
                clsx(
                  'flex items-center gap-3 px-3 py-2 rounded-md text-sm font-medium transition-colors',
                  isActive
                    ? 'bg-indigo-50 text-indigo-700'
                    : 'text-gray-700 hover:bg-gray-100',
                )
              }
            >
              <Icon className="h-4 w-4 shrink-0" />
              {label}
            </NavLink>
          ))}
        </nav>

        {/* Logout */}
        <div className="p-3 border-t border-gray-200">
          <button
            onClick={handleLogout}
            className="flex w-full items-center gap-3 px-3 py-2 rounded-md text-sm font-medium text-gray-700 hover:bg-gray-100 transition-colors"
          >
            <LogOut className="h-4 w-4 shrink-0" />
            Sign out
          </button>
        </div>
      </aside>

      {/* Main content */}
      <main className="flex-1 overflow-auto">
        <Outlet />
      </main>
    </div>
  );
}
