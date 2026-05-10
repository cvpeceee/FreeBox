import { clsx } from 'clsx';
import type { InputHTMLAttributes } from 'react';

interface InputProps extends InputHTMLAttributes<HTMLInputElement> {
  label?: string;
  error?: string;
}

export function Input({ label, error, id, className, ...props }: InputProps) {
  return (
    <div className="flex flex-col gap-1">
      {label && (
        <label htmlFor={id} className="text-sm font-medium text-gray-700">
          {label}
        </label>
      )}
      <input
        id={id}
        className={clsx(
          'block w-full rounded-md border px-3 py-2 text-sm shadow-sm',
          'placeholder:text-gray-400 focus:outline-none focus:ring-2 focus:ring-indigo-500',
          error
            ? 'border-red-500 focus:ring-red-500'
            : 'border-gray-300 focus:border-indigo-500',
          className,
        )}
        {...props}
      />
      {error && <p className="text-xs text-red-600">{error}</p>}
    </div>
  );
}
