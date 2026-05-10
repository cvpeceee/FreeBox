export function Spinner({ className }: { className?: string }) {
  return (
    <div
      role="status"
      aria-label="Loading"
      className={`h-6 w-6 border-2 border-indigo-600 border-t-transparent rounded-full animate-spin ${className ?? ''}`}
    />
  );
}
