import { Tabs, TabsList, TabsTrigger } from '@tangle/blueprint-ui/components';
import { cn } from '@tangle/blueprint-ui';

export interface ResourceTab<T extends string = string> {
  key: T;
  label: string;
  icon?: string;
  hidden?: boolean;
  disabled?: boolean;
}

interface ResourceTabsProps<T extends string = string> {
  tabs: ResourceTab<T>[];
  value: T;
  onValueChange: (value: T) => void;
  className?: string;
}

export function ResourceTabs<T extends string = string>({
  tabs,
  value,
  onValueChange,
  className,
}: ResourceTabsProps<T>) {
  const visibleTabs = tabs.filter((tab) => !tab.hidden);

  return (
    <Tabs value={value} onValueChange={(next) => onValueChange(next as T)} className={className}>
      <TabsList className="h-auto w-full justify-start gap-1 rounded-none border-b border-cloud-elements-dividerColor bg-transparent p-0">
        {visibleTabs.map((tab) => (
          <TabsTrigger
            key={tab.key}
            value={tab.key}
            disabled={tab.disabled}
            className={cn(
              'mb-[-1px] rounded-none border-b-2 border-transparent bg-transparent px-3 py-2 text-sm font-display font-medium shadow-none',
              'text-cloud-elements-textTertiary hover:text-cloud-elements-textSecondary',
              'data-[state=active]:border-violet-500 data-[state=active]:bg-transparent',
              'data-[state=active]:text-cloud-elements-textPrimary data-[state=active]:shadow-none',
              tab.disabled && 'cursor-not-allowed opacity-50',
            )}
          >
            {tab.icon ? <span className={cn(tab.icon, 'text-sm')} /> : null}
            {tab.label}
          </TabsTrigger>
        ))}
      </TabsList>
    </Tabs>
  );
}
