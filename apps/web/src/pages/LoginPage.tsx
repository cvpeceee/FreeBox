import { useState, useEffect, type FormEvent } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { auth, hashPassword } from '@/lib/api';
import { useAuthStore } from '@/stores/authStore';
import { Button } from '@/components/ui/Button';
import { Input } from '@/components/ui/Input';
import { Card, CardBody } from '@/components/ui/Card';

const PROVIDER_LABELS: Record<string, string> = {
  github: 'GitHub',
  google: 'Google',
  microsoft: 'Microsoft',
  apple: 'Apple',
  facebook: 'Facebook',
};

export default function LoginPage() {
  const navigate = useNavigate();
  const setAuthenticated = useAuthStore((s) => s.setAuthenticated);

  const [username, setUsername] = useState('');
  const [password, setPassword] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [configuredProviders, setConfiguredProviders] = useState<string[]>([]);

  useEffect(() => {
    auth.configuredProviders().then(setConfiguredProviders).catch(() => {});
  }, []);

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    setError(null);
    setLoading(true);
    try {
      // Step 1: fetch the user's stored salt (does not reveal whether user exists
      // to unauthenticated parties in a meaningful way — salt is non-secret).
      const salt = await auth.getSalt(username);
      // Step 2: derive the same hash the client used at registration.
      // The raw password is never sent over the network.
      const password_hash = await hashPassword(password, salt);
      await auth.login({ username, password_hash });
      setAuthenticated(true);
      navigate('/');
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setLoading(false);
    }
  }

  function handleOAuth(provider: string) {
    // Redirect to the server-side OAuth initiation endpoint.
    window.location.href = auth.oauthUrl(provider);
  }

  return (
    <div className="min-h-screen bg-gray-50 flex items-center justify-center p-4">
      <div className="w-full max-w-md space-y-6">
        {/* Logo / title */}
        <div className="text-center">
          <h1 className="text-3xl font-bold text-gray-900">FreeBox</h1>
          <p className="mt-2 text-sm text-gray-600">Sign in to your encrypted vault</p>
        </div>

        <Card>
          <CardBody>
            <form onSubmit={handleSubmit} className="space-y-4">
              <Input
                id="username"
                label="Username"
                type="text"
                autoComplete="username"
                required
                value={username}
                onChange={(e) => setUsername(e.target.value)}
              />
              <Input
                id="password"
                label="Password"
                type="password"
                autoComplete="current-password"
                required
                value={password}
                onChange={(e) => setPassword(e.target.value)}
              />

              {error && (
                <p className="text-sm text-red-600 bg-red-50 rounded-md px-3 py-2">{error}</p>
              )}

              <Button type="submit" className="w-full" loading={loading}>
                Sign in
              </Button>
            </form>

            <div className="mt-4 flex items-center gap-2">
              <div className="flex-1 border-t border-gray-200" />
              <span className="text-xs text-gray-500">or continue with</span>
              <div className="flex-1 border-t border-gray-200" />
            </div>

            {configuredProviders.length > 0 ? (
              <div className="mt-4 grid grid-cols-3 gap-2">
                {configuredProviders.map((id) => (
                  <Button
                    key={id}
                    variant="secondary"
                    size="sm"
                    onClick={() => handleOAuth(id)}
                  >
                    {PROVIDER_LABELS[id] ?? id}
                  </Button>
                ))}
              </div>
            ) : (
              <p className="mt-3 text-center text-xs text-gray-400">
                No social login providers configured
              </p>
            )}
          </CardBody>
        </Card>

        <p className="text-center text-sm text-gray-600">
          No account?{' '}
          <Link to="/register" className="font-medium text-indigo-600 hover:text-indigo-500">
            Create one
          </Link>
        </p>
      </div>
    </div>
  );
}
