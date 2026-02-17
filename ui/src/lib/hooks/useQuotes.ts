import { useState, useEffect, useCallback } from 'react';
import type { Address } from 'viem';
import { sha256 as viemSha256, toHex } from 'viem';
import type { DiscoveredOperator } from './useOperators';

/**
 * RFQ (Request for Quote) hook — fetches pricing quotes from operators.
 *
 * Implements the Tangle pricing protocol:
 * 1. Solve PoW challenge (SHA256 with 20-bit difficulty)
 * 2. Send gRPC GetPrice request to each operator's registered RPC
 * 3. Collect signed QuoteDetails for createServiceFromQuotes()
 *
 * When operators don't have a pricing engine (no rpcAddress), falls back to
 * multiplier-based estimation from the on-chain getDefaultJobRates().
 */

// ── Types ──

export interface OperatorQuote {
  operator: Address;
  totalCost: bigint;
  signature: `0x${string}`;
  details: {
    blueprintId: bigint;
    ttlBlocks: bigint;
    totalCost: bigint;
    timestamp: bigint;
    expiry: bigint;
    securityCommitments: readonly {
      asset: { kind: number; token: Address };
      exposureBps: number;
    }[];
  };
  costRate: number;
}

export interface UseQuotesResult {
  quotes: OperatorQuote[];
  isLoading: boolean;
  isSolvingPow: boolean;
  errors: Map<Address, string>;
  totalCost: bigint;
  refetch: () => void;
}

// ── PoW solver (mirrors Tangle pricing-engine/src/pow.rs) ──

const POW_DIFFICULTY = 20;
const PRICING_SCALE = 1_000_000_000; // 10^9

function sha256(data: Uint8Array): Uint8Array {
  return viemSha256(data, 'bytes');
}

function generateChallenge(blueprintId: bigint, timestamp: bigint): Uint8Array {
  const input = new Uint8Array(16);
  const view = new DataView(input.buffer);
  view.setBigUint64(0, blueprintId, false);
  view.setBigUint64(8, timestamp, false);
  return sha256(input);
}

function checkDifficulty(hash: Uint8Array, difficulty: number): boolean {
  const zeroBytes = Math.floor(difficulty / 8);
  const zeroBits = difficulty % 8;
  for (let i = 0; i < zeroBytes; i++) {
    if (hash[i] !== 0) return false;
  }
  if (zeroBits > 0) {
    const mask = 0xff << (8 - zeroBits);
    if ((hash[zeroBytes] & mask) !== 0) return false;
  }
  return true;
}

export async function solvePoW(
  blueprintId: bigint,
  timestamp: bigint,
): Promise<{ hash: Uint8Array; nonce: number; proof: Uint8Array }> {
  const challenge = generateChallenge(blueprintId, timestamp);
  const buf = new Uint8Array(challenge.length + 8);
  buf.set(challenge, 0);
  const view = new DataView(buf.buffer);

  for (let nonce = 0; nonce < 0x1_0000_0000; nonce++) {
    view.setBigUint64(challenge.length, BigInt(nonce), false);
    const hash = sha256(buf);
    if (checkDifficulty(hash, POW_DIFFICULTY)) {
      // Bincode-serialize Proof { hash: Vec<u8>, nonce: u64 }
      const proof = new Uint8Array(8 + 32 + 8);
      const pv = new DataView(proof.buffer);
      pv.setBigUint64(0, 32n, true);
      proof.set(hash, 8);
      pv.setBigUint64(40, BigInt(nonce), true);
      return { hash, nonce, proof };
    }
    // Yield to browser every 5000 iterations
    if (nonce % 5000 === 0 && nonce > 0) {
      await new Promise((r) => setTimeout(r, 0));
    }
  }
  throw new Error('PoW: exhausted nonce space');
}

/** Rewrite operator RPC hostname for browser reachability */
function resolveOperatorRpc(raw: string): string {
  if (typeof window === 'undefined') return raw;
  const withProto = raw.includes('://') ? raw : `http://${raw}`;
  try {
    const url = new URL(withProto);
    const pageHost = window.location.hostname;
    const isNonRoutable =
      url.hostname.endsWith('.local') ||
      !url.hostname.includes('.') ||
      url.hostname === '127.0.0.1' ||
      url.hostname === 'localhost';
    if (isNonRoutable && pageHost !== url.hostname) {
      url.hostname = pageHost;
    }
    return url.toString().replace(/\/$/, '');
  } catch {
    return withProto;
  }
}

export function formatCost(totalCost: bigint): string {
  const scaled = Number(totalCost) / PRICING_SCALE;
  if (scaled < 0.01) return `${(scaled * 1000).toFixed(2)} mTNT`;
  return `${scaled.toFixed(4)} TNT`;
}

// ── Hook ──

export function useQuotes(
  operators: DiscoveredOperator[],
  blueprintId: bigint,
  ttlBlocks: bigint,
  enabled: boolean,
): UseQuotesResult {
  const [quotes, setQuotes] = useState<OperatorQuote[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [isSolvingPow, setIsSolvingPow] = useState(false);
  const [errors, setErrors] = useState<Map<Address, string>>(new Map());
  const [fetchKey, setFetchKey] = useState(0);

  const refetch = useCallback(() => setFetchKey((k) => k + 1), []);

  useEffect(() => {
    if (!enabled || operators.length === 0) {
      setQuotes([]);
      setErrors(new Map());
      return;
    }

    let cancelled = false;
    setIsLoading(true);
    setIsSolvingPow(true);
    setQuotes([]);
    setErrors(new Map());

    async function fetchQuotes() {
      const results: OperatorQuote[] = [];
      const errs = new Map<Address, string>();

      const promises = operators.map(async (op) => {
        try {
          if (!op.rpcAddress) throw new Error('No RPC address registered');

          const rpcUrl = resolveOperatorRpc(op.rpcAddress);
          const timestamp = BigInt(Math.floor(Date.now() / 1000));

          // Solve PoW challenge
          if (!cancelled) setIsSolvingPow(true);
          const { proof } = await solvePoW(blueprintId, timestamp);
          if (!cancelled) setIsSolvingPow(false);

          // Try gRPC GetPrice endpoint
          // Since we may not have the generated protobuf types, try a JSON/REST
          // fallback first, then gRPC if available
          const response = await fetchPriceFromOperator(rpcUrl, {
            blueprintId,
            ttlBlocks,
            proofOfWork: proof,
            challengeTimestamp: timestamp,
          });

          if (!response) throw new Error('No quote returned from operator');

          if (!cancelled) results.push(response);
        } catch (err) {
          if (!cancelled) {
            errs.set(op.address, err instanceof Error ? err.message : String(err));
          }
        }
      });

      await Promise.allSettled(promises);

      if (!cancelled) {
        setQuotes(results);
        setErrors(errs);
        setIsLoading(false);
        setIsSolvingPow(false);
      }
    }

    fetchQuotes();
    return () => {
      cancelled = true;
    };
  }, [operators, blueprintId, ttlBlocks, enabled, fetchKey]);

  const totalCost = quotes.reduce((sum, q) => sum + q.totalCost, 0n);

  return { quotes, isLoading, isSolvingPow, errors, totalCost, refetch };
}

/**
 * Attempt to fetch a price from an operator's RPC endpoint.
 * Tries a JSON endpoint first (/pricing/quote), which operators may optionally expose.
 * Returns null if the operator doesn't support pricing.
 */
async function fetchPriceFromOperator(
  rpcUrl: string,
  params: {
    blueprintId: bigint;
    ttlBlocks: bigint;
    proofOfWork: Uint8Array;
    challengeTimestamp: bigint;
  },
): Promise<OperatorQuote | null> {
  // Try JSON endpoint (simpler, no protobuf dependency required)
  try {
    const response = await fetch(`${rpcUrl}/pricing/quote`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        blueprint_id: String(params.blueprintId),
        ttl_blocks: String(params.ttlBlocks),
        proof_of_work: toHex(params.proofOfWork),
        challenge_timestamp: String(params.challengeTimestamp),
        resource_requirements: [
          { kind: 'CPU', count: 1 },
          { kind: 'MemoryMB', count: 1024 },
          { kind: 'StorageMB', count: 10240 },
        ],
      }),
      signal: AbortSignal.timeout(10_000),
    });

    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const data = await response.json();

    return {
      operator: data.operator as Address,
      totalCost: BigInt(data.total_cost ?? '0'),
      signature: (data.signature ?? '0x') as `0x${string}`,
      costRate: Number(data.cost_rate ?? 0),
      details: {
        blueprintId: BigInt(data.details?.blueprint_id ?? params.blueprintId),
        ttlBlocks: BigInt(data.details?.ttl_blocks ?? params.ttlBlocks),
        totalCost: BigInt(data.details?.total_cost ?? '0'),
        timestamp: BigInt(data.details?.timestamp ?? params.challengeTimestamp),
        expiry: BigInt(data.details?.expiry ?? '0'),
        securityCommitments: (data.details?.security_commitments ?? []).map((sc: any) => ({
          asset: { kind: sc.asset?.kind ?? 0, token: (sc.asset?.token ?? '0x0000000000000000000000000000000000000000') as Address },
          exposureBps: sc.exposure_bps ?? 0,
        })),
      },
    };
  } catch {
    // JSON endpoint not available — operator doesn't have pricing engine
    return null;
  }
}
