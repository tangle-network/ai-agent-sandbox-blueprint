import { describe, it, expect } from 'vitest';
import { bytesToHex, type AttestationData } from './tee';

describe('bytesToHex', () => {
  it('converts empty array to empty string', () => {
    expect(bytesToHex([])).toBe('');
  });

  it('converts single byte to zero-padded hex', () => {
    expect(bytesToHex([0])).toBe('00');
    expect(bytesToHex([1])).toBe('01');
    expect(bytesToHex([15])).toBe('0f');
    expect(bytesToHex([16])).toBe('10');
    expect(bytesToHex([255])).toBe('ff');
  });

  it('converts multi-byte array to hex string', () => {
    expect(bytesToHex([0xde, 0xad, 0xbe, 0xef])).toBe('deadbeef');
  });

  it('handles SHA-256 sized measurement (32 bytes)', () => {
    const sha256 = Array.from({ length: 32 }, (_, i) => i);
    const hex = bytesToHex(sha256);
    expect(hex).toHaveLength(64);
    expect(hex).toBe('000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f');
  });

  it('masks values to single byte (& 0xff)', () => {
    // Values > 255 should be masked to lower 8 bits
    expect(bytesToHex([256])).toBe('00');
    expect(bytesToHex([257])).toBe('01');
    expect(bytesToHex([511])).toBe('ff');
  });

  it('handles negative values via bitmask', () => {
    // -1 & 0xff = 255 = 'ff'
    expect(bytesToHex([-1])).toBe('ff');
  });

  it('produces lowercase hex', () => {
    expect(bytesToHex([0xAB, 0xCD])).toBe('abcd');
  });
});

describe('AttestationData type', () => {
  it('accepts well-formed attestation data', () => {
    const data: AttestationData = {
      tee_type: 'sgx',
      evidence: [1, 2, 3],
      measurement: [0xde, 0xad, 0xbe, 0xef],
      timestamp: 1700000000,
    };
    expect(data.tee_type).toBe('sgx');
    expect(data.evidence).toHaveLength(3);
    expect(data.measurement).toHaveLength(4);
    expect(data.timestamp).toBe(1700000000);
  });

  it('handles empty evidence and measurement', () => {
    const data: AttestationData = {
      tee_type: 'tdx',
      evidence: [],
      measurement: [],
      timestamp: 0,
    };
    expect(bytesToHex(data.evidence)).toBe('');
    expect(bytesToHex(data.measurement)).toBe('');
  });

  it('handles real-world sized attestation (SGX quote ~1k + 48-byte measurement)', () => {
    const evidence = Array.from({ length: 1116 }, (_, i) => i & 0xff);
    const measurement = Array.from({ length: 48 }, (_, i) => (i * 7) & 0xff);
    const data: AttestationData = {
      tee_type: 'sgx',
      evidence,
      measurement,
      timestamp: Math.floor(Date.now() / 1000),
    };
    expect(bytesToHex(data.evidence)).toHaveLength(1116 * 2);
    expect(bytesToHex(data.measurement)).toHaveLength(48 * 2);
  });
});
