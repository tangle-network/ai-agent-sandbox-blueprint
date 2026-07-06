import { useStore } from '@nanostores/react';
import { Button } from '@tangle-network/blueprint-ui/components';
import { Badge } from '@tangle-network/blueprint-ui/components';
import { infraStore } from '@tangle-network/blueprint-ui';
import { BlueprintBadgeInline } from './InfraSummaryBits';

/**
 * Compact infrastructure bar shown at the top of the wizard.
 * Shows current blueprint + service, with a "Change" button to open the modal.
 */
export function InfraBar({ onOpenModal }: { onOpenModal: () => void }) {
  const infra = useStore(infraStore);

  return (
    <div className="glass-card rounded-lg p-3 flex items-center justify-between mb-6">
      <div className="flex items-center gap-4">
        <BlueprintBadgeInline blueprintId={infra.blueprintId} />
        <div className="flex items-center gap-2">
          <div className="i-ph:cpu text-sm text-cloud-elements-textTertiary" />
          <span className="text-xs text-cloud-elements-textTertiary">Service</span>
          <Badge variant={infra.serviceValidated ? 'running' : 'secondary'}>
            #{infra.serviceId}
          </Badge>
          {infra.serviceValidated && infra.serviceInfo && (
            <span className="text-xs text-cloud-elements-textTertiary">
              ({infra.serviceInfo.operatorCount} operators)
            </span>
          )}
          {!infra.serviceValidated && (
            <div className="i-ph:warning text-xs text-amber-400" title="Service not validated" />
          )}
        </div>
      </div>
      <Button variant="ghost" size="sm" onClick={onOpenModal}>
        Change
      </Button>
    </div>
  );
}
