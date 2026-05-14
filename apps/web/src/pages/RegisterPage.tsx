import { useState, type FormEvent } from 'react';
import { Link, useNavigate } from 'react-router-dom';
import { auth, hashPassword } from '@/lib/api';
import { useAuthStore } from '@/stores/authStore';
import { Button } from '@/components/ui/Button';
import { Input } from '@/components/ui/Input';
import { Card, CardBody } from '@/components/ui/Card';

/**
 * Generate a cryptographically valid prekey bundle using the Web Crypto API.
 *
 * - Ed25519 identity key pair (signing)
 * - X25519 signed prekey — public key signed by the identity private key
 * - 10 X25519 one-time prekeys
 *
 * This satisfies the server's signature verification. The private keys are
 * discarded here — in a full E2EE implementation they would be stored in
 * IndexedDB or the WASM crypto module.
 */
async function generatePrekeyBundle() {
  // Ed25519 identity key pair
  const identityKP = await crypto.subtle.generateKey(
    { name: 'Ed25519' },
    true,
    ['sign', 'verify'],
  );
  const identityPubRaw = await crypto.subtle.exportKey('raw', identityKP.publicKey);
  const identityKey = Array.from(new Uint8Array(identityPubRaw));

  // X25519 signed prekey
  const signedKP = await crypto.subtle.generateKey(
    { name: 'X25519' },
    true,
    ['deriveKey', 'deriveBits'],
  );
  const signedPubRaw = await crypto.subtle.exportKey('raw', signedKP.publicKey);
  const signedPubBytes = new Uint8Array(signedPubRaw);

  // Sign the prekey public key with the Ed25519 identity private key
  const signatureRaw = await crypto.subtle.sign(
    { name: 'Ed25519' },
    identityKP.privateKey,
    signedPubBytes,
  );

  const created_at = Math.floor(Date.now() / 1000);

  // 10 X25519 one-time prekeys
  const one_time_prekeys: { id: number; public_key: number[] }[] = [];
  for (let id = 0; id < 10; id++) {
    const kp = await crypto.subtle.generateKey({ name: 'X25519' }, true, [
      'deriveKey',
      'deriveBits',
    ]);
    const pubRaw = await crypto.subtle.exportKey('raw', kp.publicKey);
    one_time_prekeys.push({ id, public_key: Array.from(new Uint8Array(pubRaw)) });
  }

  return {
    identity_key: identityKey,
    signed_prekey: {
      public_key: Array.from(signedPubBytes),
      signature: Array.from(new Uint8Array(signatureRaw)),
      created_at,
    },
    one_time_prekeys,
  };
}

export default function RegisterPage() {
  const navigate = useNavigate();
  const setAuthenticated = useAuthStore((s) => s.setAuthenticated);

  const [form, setForm] = useState({ username: '', email: '', password: '', confirm: '' });
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  function set(field: keyof typeof form) {
    return (e: React.ChangeEvent<HTMLInputElement>) =>
      setForm((prev) => ({ ...prev, [field]: e.target.value }));
  }

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    setError(null);

    if (form.password !== form.confirm) {
      setError('Passwords do not match');
      return;
    }

    setLoading(true);
    try {
      // Generate a 16-byte random salt and encode it in Argon2's base64 alphabet
      // (A-Za-z0-9+/) with no padding — compatible with SaltString::from_b64 in the CLI.
      const saltBytes = crypto.getRandomValues(new Uint8Array(16));
      const argon2_salt = btoa(String.fromCharCode(...saltBytes))
        .replace(/\+/g, '.')   // Argon2 uses '.' instead of '+'
        .replace(/\//g, '/')   // '/' is valid in Argon2 b64
        .replace(/=+$/, '');   // no padding
      const password_hash = await hashPassword(form.password, argon2_salt);
      const prekey_bundle = await generatePrekeyBundle();
      await auth.register({
        username: form.username,
        email: form.email,
        password_hash,
        argon2_salt,
        prekey_bundle,
      });
      setAuthenticated(true);
      navigate('/');
    } catch (err) {
      setError((err as Error).message);
    } finally {
      setLoading(false);
    }
  }

  return (
    <div className="min-h-screen bg-gray-50 flex items-center justify-center p-4">
      <div className="w-full max-w-md space-y-6">
        <div className="text-center">
          <h1 className="text-3xl font-bold text-gray-900">FreeBox</h1>
          <p className="mt-2 text-sm text-gray-600">Create your encrypted vault</p>
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
                value={form.username}
                onChange={set('username')}
              />
              <Input
                id="email"
                label="Email"
                type="email"
                autoComplete="email"
                required
                value={form.email}
                onChange={set('email')}
              />
              <Input
                id="password"
                label="Password"
                type="password"
                autoComplete="new-password"
                required
                value={form.password}
                onChange={set('password')}
              />
              <Input
                id="confirm"
                label="Confirm Password"
                type="password"
                autoComplete="new-password"
                required
                value={form.confirm}
                onChange={set('confirm')}
              />

              {error && (
                <p className="text-sm text-red-600 bg-red-50 rounded-md px-3 py-2">{error}</p>
              )}

              <Button type="submit" className="w-full" loading={loading}>
                Create account
              </Button>
            </form>
          </CardBody>
        </Card>

        <p className="text-center text-sm text-gray-600">
          Already have an account?{' '}
          <Link to="/login" className="font-medium text-indigo-600 hover:text-indigo-500">
            Sign in
          </Link>
        </p>
      </div>
    </div>
  );
}
