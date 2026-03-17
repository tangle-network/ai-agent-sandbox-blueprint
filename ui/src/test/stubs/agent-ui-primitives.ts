export function truncateAddress(value: string, start = 6, end = 4): string {
  if (!value || value.length <= start + end) return value;
  return `${value.slice(0, start)}...${value.slice(-end)}`;
}

export function timeAgo(): string {
  return 'just now';
}

export function useDropdownMenu() {
  return {
    open: false,
    setOpen: () => {},
    triggerRef: { current: null },
    menuRef: { current: null },
  };
}
