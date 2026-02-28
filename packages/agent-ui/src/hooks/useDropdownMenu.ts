import { useEffect, useRef, useState } from 'react';

interface UseDropdownMenuOptions {
  closeOnEsc?: boolean;
}

export function useDropdownMenu(options?: UseDropdownMenuOptions) {
  const closeOnEsc = options?.closeOnEsc ?? true;
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    function handleClick(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) {
        setOpen(false);
      }
    }
    if (open) {
      document.addEventListener('mousedown', handleClick);
    }
    return () => document.removeEventListener('mousedown', handleClick);
  }, [open]);

  useEffect(() => {
    if (!open || !closeOnEsc) return;
    function handleKey(e: KeyboardEvent) {
      if (e.key === 'Escape') setOpen(false);
    }
    document.addEventListener('keydown', handleKey);
    return () => document.removeEventListener('keydown', handleKey);
  }, [open, closeOnEsc]);

  return {
    open,
    setOpen,
    ref,
    toggle: () => setOpen((prev) => !prev),
    close: () => setOpen(false),
  };
}
