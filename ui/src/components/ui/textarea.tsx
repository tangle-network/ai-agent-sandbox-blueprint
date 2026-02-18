import * as React from 'react';
import { cn } from '~/lib/utils';

function Textarea({ className, ...props }: React.ComponentProps<'textarea'>) {
  return (
    <textarea
      data-slot="textarea"
      className={cn(
        'min-h-[80px] w-full rounded-lg px-3.5 py-2.5 text-base font-body resize-y',
        'bg-cloud-elements-background-depth-3 dark:bg-cloud-elements-background-depth-4 border border-cloud-elements-borderColor text-cloud-elements-textPrimary',
        'placeholder:text-cloud-elements-textTertiary',
        'transition-all duration-200 outline-none',
        'hover:border-cloud-elements-borderColorActive/40',
        'focus-visible:border-violet-500/40 focus-visible:ring-2 focus-visible:ring-violet-500/10',
        'disabled:pointer-events-none disabled:opacity-40',
        className,
      )}
      {...props}
    />
  );
}

export { Textarea };
