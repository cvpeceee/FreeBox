/**
 * Global authentication state managed by Zustand.
 *
 * The store is the single source of truth for whether the user is logged in.
 * Token persistence is handled by the API client (localStorage); this store
 * only needs to know whether a valid token exists so components can react.
 */

import { create } from 'zustand';
import { getAccessToken, clearTokens } from '@/lib/api';

interface AuthState {
  /** true if an access token exists in localStorage */
  isAuthenticated: boolean;
  /** Mark the user as logged in (token already persisted by API client) */
  setAuthenticated: (value: boolean) => void;
  /** Clear tokens and mark the user as logged out */
  logout: () => void;
}

export const useAuthStore = create<AuthState>((set) => ({
  isAuthenticated: !!getAccessToken(),

  setAuthenticated: (value) => set({ isAuthenticated: value }),

  logout: () => {
    clearTokens();
    set({ isAuthenticated: false });
  },
}));
