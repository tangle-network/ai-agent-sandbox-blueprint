use super::*;

pub(crate) fn existing_store_entry_for_override(sandbox_id: &str) -> Result<Option<SandboxRecord>> {
    sandboxes()?.get(sandbox_id)
}

pub(crate) fn adjusted_sandbox_count_for_limit(
    current: usize,
    reusing_existing_slot: bool,
) -> usize {
    if reusing_existing_slot {
        current.saturating_sub(1)
    } else {
        current
    }
}

/// Global creation permit — serializes the count-check + container-create
/// sequence to prevent TOCTOU races where N concurrent creates all pass the
/// count limit check and then all succeed, exceeding the configured maximum.
///
/// The permit is held from count check through store insertion. Other
/// lifecycle operations (stop, resume) use the per-sandbox lock and do NOT
/// contend on this.
pub(crate) static CREATION_PERMIT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Acquire the creation permit. Must be held across the count check AND the
/// container creation + store insert sequence.
pub async fn acquire_creation_permit() -> tokio::sync::MutexGuard<'static, ()> {
    CREATION_PERMIT.lock().await
}

/// Decision core of the sandbox count cap, separated from store access so the
/// rejection class is unit-testable. `max == 0` = no cap.
pub(crate) fn check_sandbox_count_limit(
    current: usize,
    reusing_existing_slot: bool,
    max: usize,
) -> Result<()> {
    if max == 0 {
        return Ok(());
    }

    let effective_current = adjusted_sandbox_count_for_limit(current, reusing_existing_slot);
    if effective_current >= max {
        // Unavailable (→ 503), not Validation (→ 400): the request is
        // well-formed — this host is at capacity. The class tells callers to
        // retry on another operator instead of fixing the request.
        return Err(SandboxError::Unavailable(format!(
            "Sandbox limit reached ({current}/{max}). Delete unused sandboxes before creating new ones.",
        )));
    }

    Ok(())
}

/// One-pass scan of the store's records for admission: total row count,
/// whether the incoming create replaces an existing slot, and the running
/// set's memory + CPU footprints. Pure over a record slice so it is
/// unit-testable without a store; decisions stay in
/// [`check_sandbox_count_limit`], [`check_host_memory_budget`], and
/// [`check_host_cpu_budget`], which are unchanged.
pub(crate) struct AdmissionScan {
    pub(crate) total_count: usize,
    pub(crate) reusing_existing_slot: bool,
    pub(crate) running_memory_mb: Vec<u64>,
    pub(crate) running_cpu_cores: Vec<u64>,
}

pub(crate) fn scan_records_for_admission(
    records: &[SandboxRecord],
    reused_sandbox_id: Option<&str>,
) -> AdmissionScan {
    let mut scan = AdmissionScan {
        total_count: records.len(),
        reusing_existing_slot: false,
        running_memory_mb: Vec::with_capacity(records.len()),
        running_cpu_cores: Vec::with_capacity(records.len()),
    };
    for record in records {
        // Store keys always equal record ids (every insert uses the record's
        // id as its key), so id equality here is the same signal as the
        // former per-backend `store.get(sandbox_id).is_some()` check.
        if reused_sandbox_id == Some(record.id.as_str()) {
            // A create that replaces an existing record (recreate / image
            // upgrade) frees the old container's memory and CPU — excluded
            // from the running sums — and the count cap treats the slot as
            // reused.
            scan.reusing_existing_slot = true;
            continue;
        }
        if record.state == SandboxState::Running {
            scan.running_memory_mb.push(record.memory_mb);
            scan.running_cpu_cores.push(record.cpu_cores);
        }
    }
    scan
}

/// Sandbox count cap + host memory budget + host CPU budget from ONE store
/// read, under [`CREATION_PERMIT`].
///
/// Replaces the former `enforce_sandbox_count_limit` (called per backend) +
/// `enforce_host_memory_budget` + `enforce_host_cpu_budget` (called at
/// admission) trio, which each deserialized the full store per create. Same
/// decisions, same error precedence: memory budget, then CPU budget, then
/// the count check the backends used to run last. When no limit is
/// configured the store is not read at all.
pub(crate) fn enforce_store_admission(
    config: &SidecarRuntimeConfig,
    incoming_memory_mb: u64,
    incoming_cpu_cores: u64,
    reused_sandbox_id: Option<&str>,
) -> Result<()> {
    let memory_budget_enabled = config.sandbox_host_memory_budget_mb != 0;
    let cpu_budget_enabled = config.sandbox_host_cpu_budget != 0;
    let count_capped = config.sandbox_max_count != 0;
    if !memory_budget_enabled && !cpu_budget_enabled && !count_capped {
        return Ok(());
    }

    let records = sandboxes()?.values()?;
    let scan = scan_records_for_admission(&records, reused_sandbox_id);

    if memory_budget_enabled {
        // The warm pool's standing footprint (templates + pre-restored
        // entries) never enters the store, so reserve it here or an enabled
        // pool silently over-commits host RAM. Only read when the budget is
        // on — zero-cost otherwise, as before. (No CPU analogue: warm VMs
        // pin host RAM but time-share CPU.)
        let reserved_mb = crate::firecracker_warm::reserved_host_memory_mb()?;
        check_host_memory_budget(
            scan.running_memory_mb,
            incoming_memory_mb,
            config.sandbox_max_memory_mb,
            config.sandbox_host_memory_budget_mb,
            reserved_mb,
        )?;
    }

    if cpu_budget_enabled {
        check_host_cpu_budget(
            scan.running_cpu_cores,
            incoming_cpu_cores,
            config.sandbox_max_cpu_cores,
            config.sandbox_host_cpu_budget,
        )?;
    }

    check_sandbox_count_limit(
        scan.total_count,
        scan.reusing_existing_slot,
        config.sandbox_max_count,
    )
}

/// Apply a per-sandbox operator maximum to one requested resource value.
///
/// `max == 0` means no cap: the request passes through, including 0 =
/// unlimited. With a cap set: a request above the cap is a typed
/// `Unavailable` rejection (retry on a bigger operator), and a request of 0
/// clamps to the cap — an operator who sets a maximum must never run an
/// unlimited container.
pub(crate) fn enforce_resource_max(requested: u64, max: u64, resource: &str) -> Result<u64> {
    if max == 0 {
        return Ok(requested);
    }
    if requested == 0 {
        return Ok(max);
    }
    if requested > max {
        return Err(SandboxError::Unavailable(format!(
            "Requested {resource} {requested} exceeds this operator's maximum {max}. \
             Retry on an operator with a higher {resource} limit."
        )));
    }
    Ok(requested)
}

/// Memory a sandbox is accounted at for the host memory budget.
///
/// `None` means the footprint is unknowable: the sandbox requests unlimited
/// memory (0) and no `SANDBOX_MAX_MEMORY_MB` clamp is configured — callers
/// skip it (with a one-time warning) rather than guessing.
pub(crate) fn accounted_memory_mb(memory_mb: u64, sandbox_max_memory_mb: u64) -> Option<u64> {
    if memory_mb > 0 {
        Some(memory_mb)
    } else if sandbox_max_memory_mb > 0 {
        Some(sandbox_max_memory_mb)
    } else {
        None
    }
}

pub(crate) static UNACCOUNTABLE_MEMORY_WARN: std::sync::Once = std::sync::Once::new();

/// Decision core of the host memory budget, separated from store access so it
/// is unit-testable. `budget_mb == 0` disables the check. Records (and an
/// incoming request) with unknowable footprint are skipped with a one-time
/// warning — see [`accounted_memory_mb`].
pub(crate) fn check_host_memory_budget(
    running_memory_mb: impl IntoIterator<Item = u64>,
    incoming_memory_mb: u64,
    sandbox_max_memory_mb: u64,
    budget_mb: u64,
    reserved_mb: u64,
) -> Result<()> {
    if budget_mb == 0 {
        return Ok(());
    }

    let mut live_mb: u64 = 0;
    let mut unaccounted = 0usize;
    for memory_mb in running_memory_mb {
        match accounted_memory_mb(memory_mb, sandbox_max_memory_mb) {
            Some(mb) => live_mb = live_mb.saturating_add(mb),
            None => unaccounted += 1,
        }
    }
    let incoming_mb = match accounted_memory_mb(incoming_memory_mb, sandbox_max_memory_mb) {
        Some(mb) => mb,
        None => {
            unaccounted += 1;
            0
        }
    };
    if unaccounted > 0 {
        UNACCOUNTABLE_MEMORY_WARN.call_once(|| {
            tracing::warn!(
                unaccounted,
                "Host memory budget cannot account for sandboxes with unlimited memory; \
                 set SANDBOX_MAX_MEMORY_MB so every sandbox has a bounded footprint"
            );
        });
    }

    let committed = live_mb
        .saturating_add(incoming_mb)
        .saturating_add(reserved_mb);
    if committed > budget_mb {
        return Err(SandboxError::Unavailable(format!(
            "Host memory budget exceeded: {committed} MB committed ({live_mb} MB running + \
             {incoming_mb} MB requested + {reserved_mb} MB warm-pool reserved) > \
             SANDBOX_HOST_MEMORY_BUDGET_MB={budget_mb}. Retry on another operator."
        )));
    }

    Ok(())
}

/// CPU cores a sandbox is accounted at for the host CPU budget.
///
/// Symmetric with [`accounted_memory_mb`]: `None` means the footprint is
/// unknowable — the sandbox requests unlimited CPU (0) and no
/// `SANDBOX_MAX_CPU_CORES` clamp is configured — callers skip it (with a
/// one-time warning) rather than guessing.
pub(crate) fn accounted_cpu_cores(cpu_cores: u64, sandbox_max_cpu_cores: u64) -> Option<u64> {
    if cpu_cores > 0 {
        Some(cpu_cores)
    } else if sandbox_max_cpu_cores > 0 {
        Some(sandbox_max_cpu_cores)
    } else {
        None
    }
}

pub(crate) static UNACCOUNTABLE_CPU_WARN: std::sync::Once = std::sync::Once::new();

/// Decision core of the host CPU budget, separated from store access so it is
/// unit-testable. `budget == 0` disables the check. Records (and an incoming
/// request) with unknowable footprint are skipped with a one-time warning —
/// see [`accounted_cpu_cores`]. Mirrors [`check_host_memory_budget`]; there is
/// no CPU analogue of the warm-pool memory reservation because warm VMs pin
/// host RAM but time-share CPU.
pub(crate) fn check_host_cpu_budget(
    running_cpu_cores: impl IntoIterator<Item = u64>,
    incoming_cpu_cores: u64,
    sandbox_max_cpu_cores: u64,
    budget: u64,
) -> Result<()> {
    if budget == 0 {
        return Ok(());
    }

    let mut live_cores: u64 = 0;
    let mut unaccounted = 0usize;
    for cpu_cores in running_cpu_cores {
        match accounted_cpu_cores(cpu_cores, sandbox_max_cpu_cores) {
            Some(cores) => live_cores = live_cores.saturating_add(cores),
            None => unaccounted += 1,
        }
    }
    let incoming = match accounted_cpu_cores(incoming_cpu_cores, sandbox_max_cpu_cores) {
        Some(cores) => cores,
        None => {
            unaccounted += 1;
            0
        }
    };
    if unaccounted > 0 {
        UNACCOUNTABLE_CPU_WARN.call_once(|| {
            tracing::warn!(
                unaccounted,
                "Host CPU budget cannot account for sandboxes with unlimited CPU; \
                 set SANDBOX_MAX_CPU_CORES so every sandbox has a bounded footprint"
            );
        });
    }

    let committed = live_cores.saturating_add(incoming);
    if committed > budget {
        return Err(SandboxError::Unavailable(format!(
            "Host CPU budget exceeded: {committed} cores committed ({live_cores} running + \
             {incoming} requested) > SANDBOX_HOST_CPU_BUDGET={budget}. Retry on another operator."
        )));
    }

    Ok(())
}

/// Per-sandbox resource maxima + single-pass store admission (host memory
/// budget, host CPU budget, and sandbox count cap), applied under
/// [`CREATION_PERMIT`] before backend dispatch. Returns the request with
/// effective (possibly clamped) resource values so the container, the stored
/// record, and the budget accounting all agree.
pub(crate) fn admit_sandbox_resources(
    config: &SidecarRuntimeConfig,
    request: &CreateSandboxParams,
    sandbox_id_override: Option<&str>,
) -> Result<CreateSandboxParams> {
    let mut admitted = request.clone();
    admitted.cpu_cores =
        enforce_resource_max(request.cpu_cores, config.sandbox_max_cpu_cores, "cpu_cores")?;
    admitted.memory_mb =
        enforce_resource_max(request.memory_mb, config.sandbox_max_memory_mb, "memory_mb")?;
    admitted.disk_gb =
        enforce_resource_max(request.disk_gb, config.sandbox_max_disk_gb, "disk_gb")?;
    enforce_store_admission(
        config,
        admitted.memory_mb,
        admitted.cpu_cores,
        sandbox_id_override,
    )?;
    Ok(admitted)
}

pub(crate) fn restore_previous_store_entry(
    sandbox_id: &str,
    previous_record: Option<SandboxRecord>,
) -> Result<()> {
    match previous_record {
        Some(record) => sandboxes()?.insert(sandbox_id.to_string(), record),
        None => {
            let _ = sandboxes()?.remove(sandbox_id)?;
            Ok(())
        }
    }
}
