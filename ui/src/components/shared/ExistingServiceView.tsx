import { Input } from '@tangle-network/blueprint-ui/components';
import { Button } from '@tangle-network/blueprint-ui/components';
import { Badge } from '@tangle-network/blueprint-ui/components';
import { cn } from '@tangle-network/blueprint-ui';
import type { ServiceInfo } from '@tangle-network/blueprint-ui';
import { OperatorIdentity } from '~/components/shared/VisualIdentity';

interface ExistingServiceViewProps {
  serviceId: string;
  setServiceId: (value: string) => void;
  resetValidation: () => void;
  handleVerify: () => void;
  isValidating: boolean;
  validationError: string | null;
  serviceInfo: ServiceInfo | null;
}

export function ExistingServiceView({
  serviceId,
  setServiceId,
  resetValidation,
  handleVerify,
  isValidating,
  validationError,
  serviceInfo,
}: ExistingServiceViewProps) {
  return (
    <div className="space-y-4">
      <div className="flex gap-2">
        <Input
          type="number"
          value={serviceId}
          onChange={(e) => {
            setServiceId(e.target.value);
            resetValidation();
          }}
          placeholder="Service ID"
          min={0}
        />
        <Button
          variant="secondary"
          onClick={handleVerify}
          disabled={isValidating || !serviceId}
        >
          {isValidating ? 'Checking...' : 'Verify'}
        </Button>
      </div>

      {validationError && (
        <div className="glass-card rounded-lg p-3 border-crimson-500/30">
          <p className="text-xs text-crimson-400">{validationError}</p>
        </div>
      )}

      {serviceInfo && (
        <div className="glass-card rounded-lg p-4 space-y-3">
          <div className="flex items-center gap-2">
            <div className={cn(
              'w-2 h-2 rounded-full',
              serviceInfo.active ? 'bg-teal-400' : 'bg-crimson-400',
            )} />
            <span className="text-sm font-display font-medium">
              Service #{serviceId}
            </span>
            <Badge variant={serviceInfo.active ? 'running' : 'destructive'}>
              {serviceInfo.active ? 'Active' : 'Inactive'}
            </Badge>
          </div>

          <div className="grid grid-cols-2 gap-2 text-xs">
            <div>
              <span className="text-cloud-elements-textTertiary">Owner</span>
              <p className="font-data text-cloud-elements-textPrimary truncate">{serviceInfo.owner}</p>
            </div>
            <div>
              <span className="text-cloud-elements-textTertiary">Operators</span>
              <p className="font-data text-cloud-elements-textPrimary">{serviceInfo.operatorCount}</p>
            </div>
          </div>

          {serviceInfo.operators.length > 0 && (
            <div className="space-y-1">
              <span className="text-xs text-cloud-elements-textTertiary">Operator Addresses</span>
              {serviceInfo.operators.slice(0, 5).map((op) => (
                <div key={op} className="flex items-center gap-2">
                  <OperatorIdentity address={op} detail="service member" compact />
                </div>
              ))}
            </div>
          )}

          {!serviceInfo.permitted && (
            <div className="glass-card rounded p-2 border-amber-500/30">
              <p className="text-xs text-amber-400">
                Your address is not a permitted caller. You may need to be added.
              </p>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
