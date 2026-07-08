import { getBlueprint } from '@tangle-network/blueprint-ui';
import { Card, CardContent, CardHeader, CardTitle } from '@tangle-network/blueprint-ui/components';
import { LabeledValueRow } from '~/components/shared/LabeledValueRow';
import { ExposedPortsCard } from '~/components/shared/ExposedPortsCard';
import { OnChainVerificationCard } from '~/components/shared/OnChainVerificationCard';
import {
  OperatorIdenticon,
  getBlueprintIdentity,
  getImageIdentity,
  getResourceIdentity,
  getStatusIdentity,
} from '~/components/shared/VisualIdentity';
import { truncateAddress } from '~/lib/utils/truncate-address';
import type { LocalInstance } from '~/lib/stores/instances';
import type { useExposedPorts } from '~/lib/hooks/useExposedPorts';
import {
  getInstanceSandboxDisplayValue,
  getInstanceServiceDisplayValue,
} from '~/lib/instances/display';

interface OverviewTabProps {
  inst: LocalInstance;
  bpId: string;
  serviceId: bigint | null;
  ports: ReturnType<typeof useExposedPorts>;
  operatorUrl: string;
}

export function OverviewTab({ inst, bpId, serviceId, ports, operatorUrl }: OverviewTabProps) {
  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
      <Card>
        <CardHeader>
          <CardTitle>Instance Details</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <LabeledValueRow label="ID" value={inst.id} mono copyable identity={getBlueprintIdentity(bpId)} />
          <LabeledValueRow
            label="Sandbox"
            value={getInstanceSandboxDisplayValue(inst)}
            mono={!!inst.sandboxId}
            copyable={!!inst.sandboxId}
            copyValue={inst.sandboxId ?? undefined}
            identity={getStatusIdentity(inst.sandboxId ? 'running' : 'creating')}
          />
          <LabeledValueRow label="Image" value={inst.image} mono copyable identity={getImageIdentity(inst.image)} />
          <LabeledValueRow label="CPU" value={`${inst.cpuCores} cores`} identity={getResourceIdentity('cpu')} />
          <LabeledValueRow label="Memory" value={`${inst.memoryMb} MB`} identity={getResourceIdentity('memory')} />
          <LabeledValueRow label="Disk" value={`${inst.diskGb} GB`} identity={getResourceIdentity('disk')} />
          <LabeledValueRow label="Created" value={new Date(inst.createdAt).toLocaleString()} />
          <LabeledValueRow label="Blueprint" value={getBlueprint(bpId)?.name ?? bpId} identity={getBlueprintIdentity(bpId)} />
          <LabeledValueRow label="Service" value={getInstanceServiceDisplayValue(inst)} />
        </CardContent>
      </Card>
      <Card>
        <CardHeader>
          <CardTitle>Runtime Details</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          <LabeledValueRow
            label="Operator"
            value={inst.operator ? truncateAddress(inst.operator) : 'Unknown'}
            mono
            copyable={!!inst.operator}
            copyValue={inst.operator}
            leading={inst.operator ? <OperatorIdenticon address={inst.operator} size="sm" /> : undefined}
          />
          {inst.txHash && (
            <LabeledValueRow
              label="TX Hash"
              value={truncateAddress(inst.txHash)}
              mono
              copyable
              copyValue={inst.txHash}
            />
          )}
        </CardContent>
      </Card>

      {/* Exposed Ports */}
      {ports && ports.length > 0 && (
        <ExposedPortsCard
          ports={ports}
          proxyBaseUrl={`${operatorUrl}/api/sandbox/port/`}
          className="lg:col-span-2"
        />
      )}

      {/* On-Chain Verification */}
      {serviceId !== null && (
        <OnChainVerificationCard
          serviceId={serviceId}
          operator={inst.operator}
          sidecarUrl={inst.sidecarUrl}
          blueprintType={inst.teeEnabled ? 'tee-instance' : 'instance'}
          className="lg:col-span-2"
        />
      )}
    </div>
  );
}
