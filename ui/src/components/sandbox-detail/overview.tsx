import { Card, CardContent, CardHeader, CardTitle } from '@tangle-network/blueprint-ui/components';
import { LabeledValueRow } from '~/components/shared/LabeledValueRow';
import { ExposedPortsCard } from '~/components/shared/ExposedPortsCard';
import {
  OperatorIdenticon,
  getBlueprintIdentity,
  getImageIdentity,
  getResourceIdentity,
  getStatusIdentity,
} from '~/components/shared/VisualIdentity';
import { truncateAddress } from '~/lib/utils/truncate-address';
import type { LocalSandbox } from '~/lib/stores/sandboxes';
import type { useExposedPorts } from '~/lib/hooks/useExposedPorts';
import { formatBlueprintLabel, formatDuration, formatServiceId } from './helpers';

interface OverviewTabProps {
  sb: LocalSandbox;
  ports: ReturnType<typeof useExposedPorts>;
  operatorUrl: string;
}

export function OverviewTab({ sb, ports, operatorUrl }: OverviewTabProps) {
  return (
    <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
      <Card>
        <CardHeader>
          <CardTitle className="text-sm">Configuration</CardTitle>
        </CardHeader>
        <CardContent className="space-y-2.5">
          <LabeledValueRow
            label="Sandbox ID"
            value={sb.sandboxId && sb.sandboxId.length > 24 ? `${sb.sandboxId.slice(0, 20)}...${sb.sandboxId.slice(-4)}` : (sb.sandboxId || 'Pending operator provision')}
            mono={!!sb.sandboxId}
            copyable={!!sb.sandboxId}
            copyValue={sb.sandboxId ?? undefined}
            alignRight
            identity={getStatusIdentity(sb.sandboxId ? 'running' : 'creating')}
          />
          {sb.sandboxId == null && (
            <LabeledValueRow label="Draft Key" value={sb.localId} mono alignRight identity={getStatusIdentity('creating')} />
          )}
          <LabeledValueRow label="Image" value={sb.image} mono copyable alignRight identity={getImageIdentity(sb.image)} />
          <LabeledValueRow label="CPU" value={`${sb.cpuCores} cores`} alignRight identity={getResourceIdentity('cpu')} />
          <LabeledValueRow label="Memory" value={`${sb.memoryMb} MB`} alignRight identity={getResourceIdentity('memory')} />
          <LabeledValueRow label="Disk" value={`${sb.diskGb} GB`} alignRight identity={getResourceIdentity('disk')} />
          <LabeledValueRow label="Created" value={new Date(sb.createdAt).toLocaleString()} alignRight />
          <LabeledValueRow label="Blueprint" value={formatBlueprintLabel(sb.blueprintId)} alignRight identity={getBlueprintIdentity(sb.blueprintId)} />
          <LabeledValueRow label="Service ID" value={formatServiceId(sb.serviceId)} alignRight />
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="text-sm">Runtime Details</CardTitle>
        </CardHeader>
        <CardContent className="space-y-2.5">
          <LabeledValueRow
            label="Operator"
            value={sb.operator ? truncateAddress(sb.operator) : 'Unknown'}
            mono
            copyable={!!sb.operator}
            copyValue={sb.operator}
            alignRight
            leading={sb.operator ? <OperatorIdenticon address={sb.operator} size="sm" /> : undefined}
          />
          {sb.txHash && <LabeledValueRow label="TX Hash" value={truncateAddress(sb.txHash)} mono copyable copyValue={sb.txHash} alignRight />}
        </CardContent>
      </Card>

      {/* Lifecycle Limits */}
      {(sb.idleTimeoutSeconds != null || sb.maxLifetimeSeconds != null) && (
        <Card>
          <CardHeader>
            <CardTitle className="text-sm">Lifecycle Limits</CardTitle>
          </CardHeader>
          <CardContent className="space-y-2.5">
            {sb.idleTimeoutSeconds != null && (
              <LabeledValueRow label="Idle Timeout" value={formatDuration(sb.idleTimeoutSeconds)} alignRight />
            )}
            {sb.maxLifetimeSeconds != null && (
              <LabeledValueRow label="Max Lifetime" value={formatDuration(sb.maxLifetimeSeconds)} alignRight />
            )}
            {sb.lastActivityAt != null && (
              <LabeledValueRow label="Last Activity" value={new Date(sb.lastActivityAt).toLocaleString()} alignRight />
            )}
            {sb.maxLifetimeSeconds != null && sb.maxLifetimeSeconds > 0 && (
              <LabeledValueRow
                label="Expires At"
                value={(() => {
                  const expiresAt = sb.createdAt + sb.maxLifetimeSeconds * 1000;
                  return expiresAt < Date.now() ? 'Expired' : new Date(expiresAt).toLocaleString();
                })()}
                alignRight
              />
            )}
          </CardContent>
        </Card>
      )}

      {/* Exposed Ports */}
      {ports && ports.length > 0 && (
        <ExposedPortsCard
          ports={ports}
          proxyBaseUrl={`${operatorUrl}/api/sandboxes/${sb.sandboxId}/port/`}
          className="md:col-span-2"
        />
      )}
    </div>
  );
}
