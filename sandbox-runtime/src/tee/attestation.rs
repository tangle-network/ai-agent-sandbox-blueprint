//! Native TEE attestation via Linux device node ioctls.
//!
//! Generates hardware attestation evidence directly from the host's TEE device
//! (`/dev/tdx_guest` for Intel TDX, `/dev/sev-guest` for AMD SEV-SNP) without
//! proxying through the sidecar container. This makes the Direct backend
//! self-sufficient — attestation works even if the sidecar image doesn't
//! implement `/tee/attestation`.
//!
//! # Supported platforms
//!
//! | TEE    | Device             | ioctl               |
//! |--------|--------------------|---------------------|
//! | TDX    | `/dev/tdx_guest`   | `TDX_CMD_GET_REPORT0` |
//! | SEV-SNP| `/dev/sev-guest`   | `SNP_GET_REPORT`      |
//! | Nitro  | `/dev/nsm`         | Not supported (use sidecar) |

use crate::error::{Result, SandboxError};
use crate::tee::{AttestationReport, TeeType};

// ─────────────────────────────────────────────────────────────────────────────
// ioctl constants and struct layouts
// ─────────────────────────────────────────────────────────────────────────────

/// `_IOWR('T', 1, tdx_report_req)` — request a TDX TDREPORT.
/// _IOWR = 0xC000_0000 | (size << 16) | (type << 8) | nr
/// size = 1088, type = b'T' = 0x54, nr = 1
pub const TDX_CMD_GET_REPORT0: libc::c_ulong =
    0xC000_0000 | ((TDX_REPORT_REQ_SIZE as libc::c_ulong) << 16) | (0x54 << 8) | 1;

/// Size of `struct tdx_report_req`: 64 (reportdata) + 1024 (tdreport) = 1088.
pub const TDX_REPORT_REQ_SIZE: usize = 1088;
/// Offset of reportdata in `tdx_report_req`.
pub const TDX_REPORTDATA_OFFSET: usize = 0;
/// Size of the reportdata field.
pub const TDX_REPORTDATA_SIZE: usize = 64;
/// Offset of the TDREPORT in `tdx_report_req`.
pub const TDX_TDREPORT_OFFSET: usize = 64;
/// Size of the TDREPORT.
pub const TDX_TDREPORT_SIZE: usize = 1024;

/// Offset of MRTD within the TDREPORT.
/// REPORTMACSTRUCT (256) + TEE_TCB_INFO (239) + RSVD (17) + first 48 bytes of TDINFO_STRUCT.
/// = 256 + 239 + 17 = 512 from start of TDREPORT.
pub const TDX_MRTD_OFFSET_IN_REPORT: usize = 512;
/// Size of the MRTD measurement (SHA-384 = 48 bytes).
pub const TDX_MRTD_SIZE: usize = 48;

/// `_IOWR('S', 0x0, snp_guest_request_ioctl)`
/// The kernel SEV-SNP driver uses `_IOWR('S', 0x0, ...)` with a fixed 28-byte ioctl struct.
/// size = 28 (snp_guest_request_ioctl), type = b'S' = 0x53, nr = 0
pub const SNP_GET_REPORT: libc::c_ulong =
    0xC000_0000 | ((SNP_GUEST_REQ_IOCTL_SIZE as libc::c_ulong) << 16) | (0x53 << 8);

/// Size of `struct snp_guest_request_ioctl` (u8 msg_version + 7 pad + 3x u64 = 32).
pub const SNP_GUEST_REQ_IOCTL_SIZE: usize = 32;
/// Size of `struct snp_report_req`: 64 user_data + 4 vmpl + 28 reserved = 96.
pub const SNP_REPORT_REQ_SIZE: usize = 96;
/// Size of `struct snp_report_resp` (raw attestation report).
pub const SNP_REPORT_RESP_SIZE: usize = 4000;

/// Offset of the measurement (LAUNCH_DIGEST) within the SNP attestation report.
pub const SNP_MEASUREMENT_OFFSET: usize = 0x90; // 144
/// Size of the SNP measurement (SHA-384 = 48 bytes).
pub const SNP_MEASUREMENT_SIZE: usize = 48;

// ─────────────────────────────────────────────────────────────────────────────
// Repr-C structs matching kernel ABIs
// ─────────────────────────────────────────────────────────────────────────────

/// Matches `struct tdx_report_req` from the Linux TDX guest driver.
#[repr(C)]
pub struct TdxReportReq {
    /// 64-byte nonce supplied by the caller.
    pub reportdata: [u8; TDX_REPORTDATA_SIZE],
    /// 1024-byte TDREPORT filled by the kernel.
    pub tdreport: [u8; TDX_TDREPORT_SIZE],
}

impl TdxReportReq {
    fn new(report_data: &[u8; 64]) -> Self {
        let mut req = Self {
            reportdata: [0u8; TDX_REPORTDATA_SIZE],
            tdreport: [0u8; TDX_TDREPORT_SIZE],
        };
        req.reportdata.copy_from_slice(report_data);
        req
    }
}

/// Matches `struct snp_report_req` from the Linux SEV-SNP guest driver.
#[repr(C)]
pub struct SnpReportReq {
    /// 64-byte user-supplied data embedded in the report.
    pub user_data: [u8; 64],
    /// VMPL level for the request.
    pub vmpl: u32,
    /// Reserved, must be zero.
    pub reserved: [u8; 28],
}

impl SnpReportReq {
    fn new(user_data: &[u8; 64]) -> Self {
        let mut req = Self {
            user_data: [0u8; 64],
            vmpl: 0,
            reserved: [0u8; 28],
        };
        req.user_data.copy_from_slice(user_data);
        req
    }
}

/// Matches `struct snp_report_resp` — raw attestation report from the PSP.
#[repr(C)]
pub struct SnpReportResp {
    pub data: [u8; SNP_REPORT_RESP_SIZE],
}

impl Default for SnpReportResp {
    fn default() -> Self {
        Self {
            data: [0u8; SNP_REPORT_RESP_SIZE],
        }
    }
}

/// Matches `struct snp_guest_request_ioctl` from the Linux SEV-SNP guest driver.
///
/// Layout: `msg_version` (u8) + 7 bytes padding + `req_data` (u64) +
/// `resp_data` (u64) + `fw_error` (u64) = 32 bytes with `repr(C)`.
#[repr(C)]
pub struct SnpGuestRequestIoctl {
    /// Message version (must be 1).
    pub msg_version: u8,
    /// Pointer to request data (`snp_report_req`).
    pub req_data: u64,
    /// Pointer to response data (`snp_report_resp`).
    pub resp_data: u64,
    /// Firmware error code (output).
    pub fw_error: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Generate a hardware attestation report using native TEE device ioctls.
///
/// Returns an `AttestationReport` with raw evidence and extracted measurement.
/// On non-TEE hosts the device nodes won't exist, producing a clear error.
pub fn generate_native_attestation(
    tee_type: &TeeType,
    report_data: &[u8; 64],
) -> Result<AttestationReport> {
    match tee_type {
        TeeType::Tdx => generate_tdx_attestation(report_data),
        TeeType::Sev => generate_sev_attestation(report_data),
        TeeType::Nitro => Err(SandboxError::Validation(
            "Native Nitro attestation not supported, use sidecar".into(),
        )),
        TeeType::None => Err(SandboxError::Validation(
            "Cannot generate attestation for TeeType::None".into(),
        )),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// TDX attestation
// ─────────────────────────────────────────────────────────────────────────────

fn generate_tdx_attestation(report_data: &[u8; 64]) -> Result<AttestationReport> {
    let device_path = "/dev/tdx_guest";

    let fd = open_device(device_path)?;
    let _guard = FdGuard(fd);

    let mut req = TdxReportReq::new(report_data);

    // SAFETY: `req` is a repr(C) struct matching the kernel ABI, and `fd` is a
    // valid file descriptor for /dev/tdx_guest. The ioctl fills `req.tdreport`.
    let ret = unsafe { libc::ioctl(fd, TDX_CMD_GET_REPORT0, &mut req as *mut TdxReportReq) };
    if ret != 0 {
        let errno = std::io::Error::last_os_error();
        return Err(SandboxError::CloudProvider(format!(
            "TDX_CMD_GET_REPORT0 ioctl failed: {errno}. Kernel may not support TDX guest driver."
        )));
    }

    // Extract MRTD from the TDREPORT.
    let mrtd_start = TDX_MRTD_OFFSET_IN_REPORT;
    let mrtd_end = mrtd_start + TDX_MRTD_SIZE;
    let measurement = req.tdreport[mrtd_start..mrtd_end].to_vec();

    let report = AttestationReport {
        tee_type: TeeType::Tdx,
        evidence: req.tdreport.to_vec(),
        measurement,
        timestamp: crate::util::now_ts(),
    };

    super::validate_attestation_report(&report, &TeeType::Tdx)?;
    Ok(report)
}

// ─────────────────────────────────────────────────────────────────────────────
// SEV-SNP attestation
// ─────────────────────────────────────────────────────────────────────────────

fn generate_sev_attestation(report_data: &[u8; 64]) -> Result<AttestationReport> {
    let device_path = "/dev/sev-guest";

    let fd = open_device(device_path)?;
    let _guard = FdGuard(fd);

    let mut req = SnpReportReq::new(report_data);
    let mut resp = SnpReportResp::default();

    let mut ioctl_req = SnpGuestRequestIoctl {
        msg_version: 1,
        req_data: &mut req as *mut SnpReportReq as u64,
        resp_data: &mut resp as *mut SnpReportResp as u64,
        fw_error: 0,
    };

    // SAFETY: All pointers are valid stack allocations with repr(C) layout
    // matching the kernel ABI. `fd` is a valid /dev/sev-guest descriptor.
    let ret = unsafe {
        libc::ioctl(
            fd,
            SNP_GET_REPORT,
            &mut ioctl_req as *mut SnpGuestRequestIoctl,
        )
    };
    if ret != 0 {
        let errno = std::io::Error::last_os_error();
        return Err(SandboxError::CloudProvider(format!(
            "SNP_GET_REPORT ioctl failed: {errno}. Kernel may not support SEV-SNP guest driver."
        )));
    }

    if ioctl_req.fw_error != 0 {
        return Err(SandboxError::CloudProvider(format!(
            "SEV-SNP firmware error: 0x{:X}",
            ioctl_req.fw_error
        )));
    }

    // Extract measurement (LAUNCH_DIGEST) from the attestation report.
    let meas_end = SNP_MEASUREMENT_OFFSET + SNP_MEASUREMENT_SIZE;
    let measurement = resp.data[SNP_MEASUREMENT_OFFSET..meas_end].to_vec();

    let report = AttestationReport {
        tee_type: TeeType::Sev,
        evidence: resp.data.to_vec(),
        measurement,
        timestamp: crate::util::now_ts(),
    };

    super::validate_attestation_report(&report, &TeeType::Sev)?;
    Ok(report)
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Open a TEE device node with descriptive error messages.
fn open_device(path: &str) -> Result<libc::c_int> {
    let c_path = std::ffi::CString::new(path)
        .map_err(|_| SandboxError::Validation(format!("Invalid device path: {path}")))?;

    // SAFETY: `c_path` is a valid null-terminated C string.
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR) };
    if fd < 0 {
        let err = std::io::Error::last_os_error();
        return Err(match err.kind() {
            std::io::ErrorKind::NotFound => SandboxError::CloudProvider(format!(
                "TEE device {path} not found. Is this a TEE host?"
            )),
            std::io::ErrorKind::PermissionDenied => SandboxError::CloudProvider(format!(
                "Permission denied on {path}. Run as root or add device permissions."
            )),
            _ => SandboxError::CloudProvider(format!("Failed to open {path}: {err}")),
        });
    }
    Ok(fd)
}

/// RAII guard that closes a file descriptor on drop.
struct FdGuard(libc::c_int);

impl Drop for FdGuard {
    fn drop(&mut self) {
        // SAFETY: self.0 is a valid fd opened by `open_device`.
        unsafe {
            libc::close(self.0);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn tdx_report_req_layout() {
        assert_eq!(size_of::<TdxReportReq>(), TDX_REPORT_REQ_SIZE);
    }

    #[test]
    fn snp_report_req_layout() {
        assert_eq!(size_of::<SnpReportReq>(), SNP_REPORT_REQ_SIZE);
    }

    #[test]
    fn snp_report_resp_layout() {
        assert_eq!(size_of::<SnpReportResp>(), SNP_REPORT_RESP_SIZE);
    }

    #[test]
    fn snp_guest_request_ioctl_layout() {
        assert_eq!(size_of::<SnpGuestRequestIoctl>(), SNP_GUEST_REQ_IOCTL_SIZE);
    }

    #[test]
    fn tdx_ioctl_constant() {
        // _IOWR('T', 1, 1088): direction=0xC0, size=1088, type='T'=0x54, nr=1
        let expected: libc::c_ulong = 0xC000_0000 | (1088 << 16) | (0x54 << 8) | 1;
        assert_eq!(TDX_CMD_GET_REPORT0, expected);
    }

    #[test]
    fn snp_ioctl_constant() {
        // _IOWR('S', 0, 32): direction=0xC0, size=32, type='S'=0x53, nr=0
        let expected: libc::c_ulong = 0xC000_0000 | (32 << 16) | (0x53 << 8);
        assert_eq!(SNP_GET_REPORT, expected);
    }

    #[test]
    fn tdx_report_req_new_copies_report_data() {
        let mut data = [0u8; 64];
        data[0] = 0xAA;
        data[63] = 0xBB;
        let req = TdxReportReq::new(&data);
        assert_eq!(req.reportdata[0], 0xAA);
        assert_eq!(req.reportdata[63], 0xBB);
        assert_eq!(req.tdreport, [0u8; TDX_TDREPORT_SIZE]);
    }

    #[test]
    fn snp_report_req_new_copies_user_data() {
        let mut data = [0u8; 64];
        data[0] = 0xCC;
        data[63] = 0xDD;
        let req = SnpReportReq::new(&data);
        assert_eq!(req.user_data[0], 0xCC);
        assert_eq!(req.user_data[63], 0xDD);
        assert_eq!(req.vmpl, 0);
        assert_eq!(req.reserved, [0u8; 28]);
    }

    #[test]
    fn generate_native_nitro_returns_error() {
        let data = [0u8; 64];
        let result = generate_native_attestation(&TeeType::Nitro, &data);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("Nitro"), "error should mention Nitro: {err}");
    }

    #[test]
    fn generate_native_none_returns_error() {
        let data = [0u8; 64];
        let result = generate_native_attestation(&TeeType::None, &data);
        assert!(result.is_err());
    }

    #[test]
    fn tdx_device_not_found() {
        // On non-TEE hosts, /dev/tdx_guest won't exist.
        let data = [0u8; 64];
        let result = generate_native_attestation(&TeeType::Tdx, &data);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        // Should get "not found" or "permission denied", not a panic.
        assert!(
            err.contains("not found")
                || err.contains("Permission denied")
                || err.contains("Failed to open"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn sev_device_not_found() {
        let data = [0u8; 64];
        let result = generate_native_attestation(&TeeType::Sev, &data);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("not found")
                || err.contains("Permission denied")
                || err.contains("Failed to open"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn mrtd_extraction_from_known_pattern() {
        // Simulate a TDREPORT with known bytes at the MRTD offset.
        let mut fake_report = [0u8; TDX_TDREPORT_SIZE];
        for (i, byte) in fake_report
            [TDX_MRTD_OFFSET_IN_REPORT..TDX_MRTD_OFFSET_IN_REPORT + TDX_MRTD_SIZE]
            .iter_mut()
            .enumerate()
        {
            *byte = (i + 1) as u8;
        }

        let measurement = fake_report
            [TDX_MRTD_OFFSET_IN_REPORT..TDX_MRTD_OFFSET_IN_REPORT + TDX_MRTD_SIZE]
            .to_vec();
        assert_eq!(measurement.len(), 48);
        assert_eq!(measurement[0], 1);
        assert_eq!(measurement[47], 48);
    }

    #[test]
    fn snp_measurement_extraction_from_known_pattern() {
        let mut fake_resp = [0u8; SNP_REPORT_RESP_SIZE];
        for (i, byte) in fake_resp
            [SNP_MEASUREMENT_OFFSET..SNP_MEASUREMENT_OFFSET + SNP_MEASUREMENT_SIZE]
            .iter_mut()
            .enumerate()
        {
            *byte = (i + 0x10) as u8;
        }

        let measurement = fake_resp
            [SNP_MEASUREMENT_OFFSET..SNP_MEASUREMENT_OFFSET + SNP_MEASUREMENT_SIZE]
            .to_vec();
        assert_eq!(measurement.len(), 48);
        assert_eq!(measurement[0], 0x10);
        assert_eq!(measurement[47], 0x3F);
    }
}
