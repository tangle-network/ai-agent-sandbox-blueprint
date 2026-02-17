import { useCallback, useState } from 'react';
import { useAccount, useWriteContract, useWaitForTransactionReceipt } from 'wagmi';
import { tangleJobsAbi } from '~/lib/contracts/abi';
import { getAddresses } from '~/lib/contracts/publicClient';
import { addTx, updateTx } from '~/lib/stores/txHistory';
import { selectedChainIdStore } from '~/lib/contracts/publicClient';
import { PRICING_TIERS } from '~/lib/types/sandbox';
import { useEffect } from 'react';

export interface SubmitJobOpts {
  serviceId: bigint;
  jobId: number;
  args: `0x${string}`;
  label?: string;
  value?: bigint;
}

export type JobSubmitStatus = 'idle' | 'signing' | 'pending' | 'confirmed' | 'failed';

export function useSubmitJob() {
  const { address } = useAccount();
  const { writeContractAsync, data: hash, isPending: isSigning } = useWriteContract();
  const [status, setStatus] = useState<JobSubmitStatus>('idle');
  const [error, setError] = useState<string | null>(null);
  const [txHash, setTxHash] = useState<`0x${string}` | undefined>();

  const { isSuccess, isError } = useWaitForTransactionReceipt({
    hash: txHash,
  });

  useEffect(() => {
    if (isSuccess && txHash) {
      setStatus('confirmed');
      updateTx(txHash, { status: 'confirmed' });
    }
    if (isError && txHash) {
      setStatus('failed');
      updateTx(txHash, { status: 'failed' });
    }
  }, [isSuccess, isError, txHash]);

  const submitJob = useCallback(
    async (opts: SubmitJobOpts) => {
      if (!address) {
        setError('Wallet not connected');
        return undefined;
      }

      const addrs = getAddresses();
      const tier = PRICING_TIERS[opts.jobId];
      const label = opts.label ?? tier?.label ?? `Job #${opts.jobId}`;

      try {
        setStatus('signing');
        setError(null);

        const result = await writeContractAsync({
          address: addrs.jobs,
          abi: tangleJobsAbi,
          functionName: 'submitJob',
          args: [opts.serviceId, opts.jobId, opts.args],
          value: opts.value,
        });

        setTxHash(result);
        setStatus('pending');
        addTx(result, label, selectedChainIdStore.get());

        return result;
      } catch (err: any) {
        setStatus('failed');
        const msg = err?.shortMessage ?? err?.message ?? 'Transaction failed';
        setError(msg);
        return undefined;
      }
    },
    [address, writeContractAsync],
  );

  const reset = useCallback(() => {
    setStatus('idle');
    setError(null);
    setTxHash(undefined);
  }, []);

  return {
    submitJob,
    reset,
    status,
    error,
    txHash,
    isSigning,
    isConnected: !!address,
  };
}
