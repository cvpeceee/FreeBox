/**
 * OAuthCallbackPage
 *
 * The server handles the actual OAuth code exchange and issues our JWT tokens.
 * The server should redirect to the web app with tokens in the URL fragment or
 * a short-lived code that we exchange here.
 *
 * For now, this page reads tokens from the URL search params
 * (?access_token=...&refresh_token=...) which the server should set after a
 * successful OAuth callback before redirecting here.
 */

import { useEffect, useState } from 'react';
import { useNavigate, useSearchParams } from 'react-router-dom';
import { saveTokens } from '@/lib/api';
import { useAuthStore } from '@/stores/authStore';
import { Spinner } from '@/components/ui/Spinner';

export default function OAuthCallbackPage() {
  const [searchParams] = useSearchParams();
  const navigate = useNavigate();
  const setAuthenticated = useAuthStore((s) => s.setAuthenticated);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const accessToken = searchParams.get('access_token');
    const refreshToken = searchParams.get('refresh_token');
    const expiresIn = searchParams.get('expires_in');
    const errorMsg = searchParams.get('error');

    if (errorMsg) {
      setError(decodeURIComponent(errorMsg));
      return;
    }

    if (accessToken && refreshToken) {
      saveTokens({
        access_token: accessToken,
        refresh_token: refreshToken,
        expires_in: Number(expiresIn ?? 900),
      });
      setAuthenticated(true);
      navigate('/', { replace: true });
    } else {
      setError('OAuth callback did not return valid tokens. Please try again.');
    }
  }, [searchParams, navigate, setAuthenticated]);

  if (error) {
    return (
      <div className="min-h-screen bg-gray-50 flex items-center justify-center p-4">
        <div className="text-center space-y-4">
          <p className="text-red-600 font-medium">{error}</p>
          <a href="/login" className="text-indigo-600 hover:underline text-sm">
            Back to login
          </a>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-gray-50 flex items-center justify-center">
      <div className="text-center space-y-4">
        <Spinner className="mx-auto" />
        <p className="text-gray-600 text-sm">Signing you in…</p>
      </div>
    </div>
  );
}
