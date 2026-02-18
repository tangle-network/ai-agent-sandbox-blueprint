import { cn } from '~/lib/utils';

interface ToggleProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
  className?: string;
}

function Toggle({ checked, onChange, disabled, className }: ToggleProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cn(
        'relative w-11 h-6 rounded-full transition-colors',
        checked ? 'bg-violet-600' : 'bg-cloud-elements-background-depth-4',
        disabled && 'opacity-40 pointer-events-none',
        className,
      )}
    >
      <span
        className={cn(
          'absolute top-0.5 left-0.5 w-5 h-5 rounded-full bg-white transition-transform',
          checked && 'translate-x-5',
        )}
      />
    </button>
  );
}

export { Toggle };
