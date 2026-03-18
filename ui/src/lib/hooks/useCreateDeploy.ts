/**
 * useCreateDeploy — unified deploy hook for the create wizard.
 *
 * Abstracts the two deployment paths behind a single state machine:
 *   Path A (sandbox / instance with existing service): submitJob → watch provision
 *   Path B (instance without service): requestService → watch ServiceActivated → watch OperatorProvisioned
 *
 * Path A bypasses MetaMask's built-in gas estimation by pre-estimating gas
 * through publicClient (uses the working Vite RPC proxy). This avoids the
 * "Requested resource not available" error when MetaMask's internal RPC URL
 * for the chain is unreachable (common in remote/Tailscale dev setups).
 */

import { useState, useCallback, useEffect, useLayoutEffect, useMemo, useRef } from 'react';
import { useAccount, useWriteContract, useWaitForTransactionReceipt, useChainId } from 'wagmi';
import { decodeEventLog } from 'viem';
import { encodeJobArgs } from '@tangle-network/blueprint-ui';
import { tangleServicesAbi } from '@tangle-network/blueprint-ui';
import { getAddresses, publicClient } from '@tangle-network/blueprint-ui';
import { tangleJobsAbi, addTx, updateTx } from '@tangle-network/blueprint-ui';
import { useOperators, type DiscoveredOperator } from '@tangle-network/blueprint-ui';
import { selectedChainIdStore } from '@tangle-network/blueprint-ui';
import {
  deriveMode,
  computeStatus,
  computeCanDeploy,
  type DeployMode,
  type DeployStatus,
  type JobSubmitStatus,
} from './createDeployLogic';
import { useInstanceProvisionWatcher } from '~/lib/hooks/useProvisionWatcher';
import { addSandbox, createSandboxDraftId, updateSandboxStatus } from '~/lib/stores/sandboxes';
import { addInstance, updateInstanceStatus } from '~/lib/stores/instances';
import type { BlueprintDefinition, JobDefinition } from '@tangle-network/blueprint-ui';
import { isContractDeployed, type SandboxAddresses } from '~/lib/contracts/chains';
import type { InfraConfig } from '@tangle-network/blueprint-ui';
import type { Address } from 'viem';
import { expectedLocalRpcUrl, walletRpcMatchesAppRpc } from '~/lib/walletRpcSync';

// Re-export types from logic module for external consumers
export type { DeployMode, DeployStatus, JobSubmitStatus } from './createDeployLogic';

export interface ProvisionInfo {
  sandboxId: string;
  sidecarUrl: string;
}

export interface CreateDeployState {
  mode: DeployMode;
  status: DeployStatus;
  txHash?: `0x${string}`;
  error?: string;
  callId?: number;
  provision?: ProvisionInfo;
  sandboxDraftKey?: string;
  /** Discovered operators (only relevant for instance mode without existing service) */
  operators: DiscoveredOperator[];
  operatorsLoading: boolean;
  /** Whether we're creating a new service (instance Path B) vs submitting a job (Path A) */
  isNewService: boolean;
}

interface UseCreateDeployOpts {
  blueprint: BlueprintDefinition | undefined;
  job: JobDefinition | null;
  values: Record<string, unknown>;
  infra: InfraConfig;
  validate: () => boolean;
}

/** ~30 days at 3s blocks */
const TTL_BLOCKS_30_DAYS = 864000n;
const useSafeLayoutEffect = typeof window === 'undefined' ? useEffect : useLayoutEffect;

// ── Hook ──

export function useCreateDeploy({ blueprint, job, values, infra, validate }: UseCreateDeployOpts) {
  const { address, isConnected } = useAccount();
  const walletChainId = useChainId();

  const mode = deriveMode(blueprint?.id);
  const isTeeInstance = blueprint?.id === 'ai-agent-tee-instance-blueprint';
  const isInstanceMode = mode === 'instance';

  // Store latest values in refs to avoid stale closures in useEffect hooks.
  // These change on every keystroke, but the effects should only fire on their
  // specific trigger deps (callId, serviceConfirmed, instanceProvision).
  const valuesRef = useRef(values);
  valuesRef.current = values;

  const modeRef = useRef(mode);
  modeRef.current = mode;

  const infraRef = useRef(infra);
  infraRef.current = infra;

  const isTeeInstanceRef = useRef(isTeeInstance);
  isTeeInstanceRef.current = isTeeInstance;
  const sandboxDraftKeyRef = useRef<string | null>(null);

  // ── Path A: submitJob (sandbox, or instance with existing service) ──
  // Uses writeContractAsync directly with pre-estimated gas to bypass
  // MetaMask's gas estimation (which fails when its RPC URL is unreachable).
  const {
    writeContractAsync: jobWriteAsync,
    data: jobTxHash,
    isPending: jobSigning,
    reset: resetJobTx,
  } = useWriteContract();
  const {
    data: jobReceipt,
    isSuccess: jobConfirmed,
    isLoading: jobTxPending,
    isError: jobTxFailed,
  } = useWaitForTransactionReceipt({ hash: jobTxHash });

  const [jobError, setJobError] = useState<string | null>(null);

  // Extract callId from the JobCalled event in the receipt logs
  const callId = useMemo<number | null>(() => {
    if (!jobReceipt?.logs) return null;
    for (const log of jobReceipt.logs) {
      try {
        const decoded = decodeEventLog({
          abi: tangleJobsAbi,
          data: log.data,
          topics: log.topics,
        });
        if (decoded.eventName === 'JobCalled' && 'callId' in decoded.args) {
          return Number(decoded.args.callId);
        }
      } catch {
        // Not a matching event, skip
      }
    }
    return null;
  }, [jobReceipt]);

  // Derive jobStatus from the write/receipt state
  const jobStatus = useMemo<JobSubmitStatus>(() => {
    if (jobError) return 'failed';
    if (jobSigning) return 'signing';
    if (jobTxPending) return 'pending';
    if (jobTxFailed) return 'failed';
    if (jobConfirmed) return 'confirmed';
    return 'idle';
  }, [jobError, jobSigning, jobTxPending, jobTxFailed, jobConfirmed]);

  // Track job TX in the tx history store
  useEffect(() => {
    if (jobTxHash) {
      addTx(jobTxHash, `Job`, selectedChainIdStore.get());
    }
  }, [jobTxHash]);
  useEffect(() => {
    if (jobConfirmed && jobTxHash) updateTx(jobTxHash, { status: 'confirmed' });
    if (jobTxFailed && jobTxHash) updateTx(jobTxHash, { status: 'failed' });
  }, [jobConfirmed, jobTxFailed, jobTxHash]);

  // ── Path B: requestService (instance without existing service) ──
  const {
    writeContractAsync: requestServiceWrite,
    data: serviceTxHash,
    isPending: serviceSigning,
    reset: resetServiceTx,
  } = useWriteContract();
  const {
    isSuccess: serviceConfirmed,
    isLoading: serviceTxPending,
  } = useWaitForTransactionReceipt({ hash: serviceTxHash });

  // Service error tracking for Path B
  const [serviceError, setServiceError] = useState<string | null>(null);
  // Pre-flight error (simulation failure, wallet not connected, etc.)
  const [preflightError, setPreflightError] = useState<string | null>(null);

  // Operator discovery (instance mode)
  const { operators, isLoading: operatorsLoading } = useOperators(
    isInstanceMode ? BigInt(infra.blueprintId || '0') : 0n,
  );

  // Check if existing service is valid (cached in infra store)
  const hasValidService = !!(
    infra.serviceInfo?.active &&
    infra.serviceInfo?.permitted &&
    infra.serviceId
  );

  // Whether we're creating a new service (Path B) vs submitting a job (Path A)
  const isNewService = isInstanceMode && !hasValidService;

  // ── Unified status ──

  const rawStatus = useMemo<DeployStatus>(
    () => computeStatus({ isNewService, jobStatus, serviceSigning, serviceTxPending, serviceConfirmed, serviceError }),
    [isNewService, jobStatus, serviceSigning, serviceTxPending, serviceConfirmed, serviceError],
  );
  // Preflight errors (simulation, wallet) override status to 'failed'
  const status = preflightError ? 'failed' as DeployStatus : rawStatus;

  const txHash = isNewService ? serviceTxHash : jobTxHash;
  const error = preflightError ?? (isNewService ? serviceError : jobError);

  // ── Instance provision watching ──

  const instanceProvision = useInstanceProvisionWatcher(
    infra.serviceId ? BigInt(infra.serviceId) : null,
    isTeeInstance ? 'tee-instance' : 'instance',
    status === 'confirmed' && isInstanceMode,
  );

  // ── Store updates ──

  // Update store when submitJob callId is parsed from receipt
  useSafeLayoutEffect(() => {
    if (callId != null) {
      if (modeRef.current === 'sandbox') {
        if (!sandboxDraftKeyRef.current) return;
        updateSandboxStatus(sandboxDraftKeyRef.current, 'creating', { callId });
      } else {
        const name = String(valuesRef.current.name || '');
        if (!name) return;
        updateInstanceStatus(name, 'creating', { callId });
      }
    }
  }, [callId]);

  useEffect(() => {
    if (!sandboxDraftKeyRef.current || modeRef.current !== 'sandbox') return;
    if (!jobError && !jobTxFailed) return;
    updateSandboxStatus(sandboxDraftKeyRef.current, 'error', {
      errorMessage: jobError ?? 'Sandbox creation transaction failed before provisioning started.',
    });
  }, [jobError, jobTxFailed]);

  // Add instance to store when requestService confirms
  useEffect(() => {
    if (serviceConfirmed && isInstanceMode) {
      const v = valuesRef.current;
      const name = String(v.name || '');
      if (!name) return;
      addInstance({
        id: name,
        name,
        image: String(v.image || ''),
        cpuCores: Number(v.cpuCores) || 2,
        memoryMb: Number(v.memoryMb) || 2048,
        diskGb: Number(v.diskGb) || 10,
        createdAt: Date.now(),
        blueprintId: infraRef.current.blueprintId,
        serviceId: infraRef.current.serviceId || '',
        teeEnabled: isTeeInstanceRef.current,
        agentIdentifier: String(v.agentIdentifier || '') || undefined,
        status: 'creating',
        txHash: serviceTxHash,
      });
    }
  }, [serviceConfirmed, isInstanceMode, serviceTxHash]);

  // Update store when instance provision event arrives
  useEffect(() => {
    if (instanceProvision) {
      const name = String(valuesRef.current.name || '');
      if (name) {
        updateInstanceStatus(name, 'running', {
          id: instanceProvision.sandboxId,
          sidecarUrl: instanceProvision.sidecarUrl,
        });
      }
    }
  }, [instanceProvision]);

  // ── Deploy action ──

  const deploy = useCallback(async () => {
    if (!job || !validate()) return;

    // Verify wallet is fully connected (not just cached from localStorage)
    setPreflightError(null);
    setJobError(null);
    if (!isConnected || !address) {
      setPreflightError('Wallet not connected. Please connect your wallet and try again.');
      return;
    }

    const selectedChainId = selectedChainIdStore.get();
    const rpcMatches = await walletRpcMatchesAppRpc(selectedChainId);
    if (rpcMatches === false) {
      setPreflightError(
        `Wallet is connected to a different local RPC than the app. ` +
        `Update your wallet's Local network RPC URL to ${expectedLocalRpcUrl()} and try again.`,
      );
      return;
    }

    const args = encodeJobArgs(job, values);
    const name = String(values.name || '');

    // Path A: Submit job to existing service
    if (!isNewService) {
      const serviceId = BigInt(infra.serviceId || '0');
      const value = BigInt(job.pricingMultiplier ?? 50) * 1_000_000_000_000_000n;
      const addrs = getAddresses<SandboxAddresses>();

      // Pre-flight: simulate + estimate gas via our publicClient (uses working
      // Vite RPC proxy). This catches contract errors AND provides gas so
      // MetaMask doesn't need to estimate using its own (potentially broken) RPC.
      let gas: bigint;
      try {
        await publicClient.simulateContract({
          address: addrs.jobs,
          abi: tangleJobsAbi,
          functionName: 'submitJob',
          args: [serviceId, job.id, args],
          value,
          account: address as Address,
        });
        gas = await publicClient.estimateContractGas({
          address: addrs.jobs,
          abi: tangleJobsAbi,
          functionName: 'submitJob',
          args: [serviceId, job.id, args],
          value,
          account: address as Address,
        });
        // Add 20% buffer for safety
        gas = gas + (gas / 5n);
      } catch (simErr: any) {
        const simMsg = simErr?.shortMessage ?? simErr?.message ?? 'Simulation failed';
        console.error('[deploy] Pre-flight simulation failed:', simErr);
        setPreflightError(`Contract simulation failed: ${simMsg}`);
        return;
      }

      // Send to wallet with pre-estimated gas — MetaMask skips its own
      // gas estimation and goes straight to the confirmation popup.
      let submittedJobTxHash: `0x${string}`;
      try {
        submittedJobTxHash = await jobWriteAsync({
          address: addrs.jobs,
          abi: tangleJobsAbi,
          functionName: 'submitJob',
          args: [serviceId, job.id, args],
          value,
          gas,
        });
      } catch (err: any) {
        const msg = err?.shortMessage ?? err?.message ?? 'Transaction failed';
        setJobError(msg);
        return;
      }

      // Add to local store (txHash is set by jobWriteAsync via the useWriteContract hook)
      const agentIdentifier = String(values.agentIdentifier || '');
      if (mode === 'sandbox') {
        const draftKey = createSandboxDraftId(name);
        sandboxDraftKeyRef.current = draftKey;
        addSandbox({
          localId: draftKey,
          name,
          image: String(values.image || ''),
          cpuCores: Number(values.cpuCores) || 2,
          memoryMb: Number(values.memoryMb) || 2048,
          diskGb: Number(values.diskGb) || 10,
          createdAt: Date.now(),
          blueprintId: infra.blueprintId,
          serviceId: infra.serviceId,
          status: 'creating',
          txHash: submittedJobTxHash,
          agentIdentifier: agentIdentifier || undefined,
          teeEnabled: isTeeInstance || undefined,
          webTerminalEnabled: values.webTerminalEnabled !== false,
        });
      } else {
        addInstance({
          id: name,
          name,
          image: String(values.image || ''),
          cpuCores: Number(values.cpuCores) || 2,
          memoryMb: Number(values.memoryMb) || 2048,
          diskGb: Number(values.diskGb) || 10,
          createdAt: Date.now(),
          blueprintId: infra.blueprintId,
          serviceId: infra.serviceId,
          status: 'creating',
          agentIdentifier: agentIdentifier || undefined,
          teeEnabled: isTeeInstance,
        });
      }
      return;
    }

    // Path B: Create new service with config as requestInputs
    const addrs = getAddresses<SandboxAddresses>();
    const ops = operators.map((o) => o.address);
    if (ops.length === 0) return;

    try {
      setServiceError(null);

      // Pre-estimate gas for Path B too
      let gas: bigint;
      try {
        gas = await publicClient.estimateContractGas({
          address: addrs.services,
          abi: tangleServicesAbi,
          functionName: 'requestService',
          args: [
            BigInt(infra.blueprintId),
            ops,
            args as `0x${string}`,
            [address as Address],
            TTL_BLOCKS_30_DAYS,
            '0x0000000000000000000000000000000000000000' as Address,
            0n,
          ],
          account: address as Address,
        });
        gas = gas + (gas / 5n);
      } catch (estErr: any) {
        const msg = estErr?.shortMessage ?? estErr?.message ?? 'Gas estimation failed';
        console.error('[deploy] Path B gas estimation failed:', estErr);
        setServiceError(`Gas estimation failed: ${msg}`);
        return;
      }

      await requestServiceWrite({
        address: addrs.services,
        abi: tangleServicesAbi,
        functionName: 'requestService',
        args: [
          BigInt(infra.blueprintId),
          ops,
          args as `0x${string}`,
          [address as Address],
          TTL_BLOCKS_30_DAYS,
          '0x0000000000000000000000000000000000000000' as Address,
          0n,
        ],
        gas,
      });
    } catch (err: unknown) {
      const message =
        err instanceof Error
          ? (err as Error & { shortMessage?: string }).shortMessage ?? err.message
          : String(err);
      setServiceError(message || 'Service creation failed');
    }
  }, [job, values, infra, validate, isNewService, isConnected, jobWriteAsync, address, operators, requestServiceWrite, mode, isTeeInstance]);

  // ── Reset ──

  const reset = useCallback(() => {
    sandboxDraftKeyRef.current = null;
    resetJobTx();
    resetServiceTx();
    setJobError(null);
    setServiceError(null);
    setPreflightError(null);
  }, [resetJobTx, resetServiceTx]);

  // ── Computed flags ──

  // Check if contracts are deployed on the current network
  const addrs = getAddresses<SandboxAddresses>();
  const contractsDeployed = isContractDeployed(addrs.jobs) && isContractDeployed(addrs.services);

  const selectedChainId = selectedChainIdStore.get();
  const correctChain = walletChainId === selectedChainId;

  // Use rawStatus for canDeploy so preflight errors don't permanently disable the button.
  // The deploy function clears preflightError before each attempt, allowing retries.
  const canDeploy = computeCanDeploy({
    job: !!job,
    hasName: !!values.name,
    hasAddress: isConnected && !!address,
    status: rawStatus,
    contractsDeployed,
    correctChain,
    mode,
    hasValidService,
    isNewService,
    operatorCount: operators.length,
    operatorsLoading,
  });

  return {
    // State
    mode,
    status,
    txHash,
    error,
    callId: callId ?? undefined,
    provision: instanceProvision ?? undefined,
    sandboxDraftKey: sandboxDraftKeyRef.current ?? undefined,
    operators,
    operatorsLoading,
    isNewService,
    isInstanceMode,
    isTeeInstance,
    hasValidService,
    contractsDeployed,
    canDeploy,
    // Actions
    deploy,
    reset,
  };
}
