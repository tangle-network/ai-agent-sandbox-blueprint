import { Badge } from '@tangle-network/blueprint-ui/components';

export function BlueprintBadgeInline({ blueprintId }: { blueprintId: string }) {
  return (
    <div className="flex items-center gap-2">
      <div className="i-ph:cube text-sm text-cloud-elements-textTertiary" />
      <span className="text-xs text-cloud-elements-textTertiary">Blueprint</span>
      <Badge variant="accent">#{blueprintId}</Badge>
    </div>
  );
}
