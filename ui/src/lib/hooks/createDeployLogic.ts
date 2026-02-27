/**
 * Pure computation functions for useCreateDeploy.
 *
 * Separated from the hook so they can be tested without pulling in
 * wagmi, contract ABIs, or other heavy runtime dependencies.
 */

export type DeployMode = 'sandbox' | 'instance';

export type DeployStatus =
  | 'idle'
  | 'signing'
  | 'pending'
  | 'confirmed'
  | 'provisioning'
  | 'ready'
  | 'failed';

export type JobSubmitStatus = 'idle' | 'signing' | 'pending' | 'confirmed' | 'failed';

/** Derive deploy mode from blueprint ID */
export function deriveMode(blueprintId: string | undefined): DeployMode {
  return blueprintId === 'ai-agent-sandbox-blueprint' ? 'sandbox' : 'instance';
}

/** Derive whether a new service must be created (Path B) */
export function deriveIsNewService(
  mode: DeployMode,
  serviceActive: boolean,
  servicePermitted: boolean,
  serviceId: string,
): boolean {
  return mode === 'instance' && !(serviceActive && servicePermitted && serviceId);
}

/** Compute unified deploy status from Path A and Path B signals */
export function computeStatus(opts: {
  isNewService: boolean;
  jobStatus: JobSubmitStatus;
  serviceSigning: boolean;
  serviceTxPending: boolean;
  serviceConfirmed: boolean;
  serviceError: string | null;
}): DeployStatus {
  const { isNewService, jobStatus, serviceSigning, serviceTxPending, serviceConfirmed, serviceError } = opts;
  if (!isNewService) {
    if (jobStatus === 'signing') return 'signing';
    if (jobStatus === 'pending') return 'pending';
    if (jobStatus === 'failed') return 'failed';
    if (jobStatus === 'confirmed') return 'confirmed';
    return 'idle';
  }
  if (serviceError) return 'failed';
  if (serviceSigning) return 'signing';
  if (serviceTxPending) return 'pending';
  if (serviceConfirmed) return 'confirmed';
  return 'idle';
}

/** Compute canDeploy flag */
export function computeCanDeploy(opts: {
  job: boolean;
  hasName: boolean;
  hasAddress: boolean;
  status: DeployStatus;
  contractsDeployed: boolean;
  mode: DeployMode;
  hasValidService: boolean;
  isNewService: boolean;
  operatorCount: number;
  operatorsLoading: boolean;
}): boolean {
  return !!(
    opts.job &&
    opts.hasName &&
    opts.hasAddress &&
    opts.status === 'idle' &&
    opts.contractsDeployed &&
    (opts.mode === 'sandbox' ? opts.hasValidService : true) &&
    (!opts.isNewService || (opts.operatorCount > 0 && !opts.operatorsLoading))
  );
}
