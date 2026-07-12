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

pub(crate) fn enforce_sandbox_count_limit(
    config: &SidecarRuntimeConfig,
    reusing_existing_slot: bool,
) -> Result<()> {
    let current = sandboxes()?.values()?.len();
    check_sandbox_count_limit(current, reusing_existing_slot, config.sandbox_max_count)
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

/// Enforce `SANDBOX_HOST_MEMORY_BUDGET_MB` at admission. Must be called with
/// [`CREATION_PERMIT`] held so the running-memory sum cannot race a
/// concurrent create.
pub(crate) fn enforce_host_memory_budget(
    config: &SidecarRuntimeConfig,
    incoming_memory_mb: u64,
    reused_sandbox_id: Option<&str>,
) -> Result<()> {
    if config.sandbox_host_memory_budget_mb == 0 {
        return Ok(());
    }

    let running_memory_mb: Vec<u64> = sandboxes()?
        .values()?
        .into_iter()
        .filter(|record| record.state == SandboxState::Running)
        // A create that replaces an existing record (recreate / image upgrade)
        // frees the old container's memory, so it doesn't count against the budget.
        .filter(|record| reused_sandbox_id != Some(record.id.as_str()))
        .map(|record| record.memory_mb)
        .collect();

    // The warm pool's standing footprint (templates + pre-restored entries)
    // never enters the store, so reserve it here or an enabled pool silently
    // over-commits host RAM. Zero when warm serving is disabled.
    let reserved_mb = crate::firecracker_warm::reserved_host_memory_mb()?;

    check_host_memory_budget(
        running_memory_mb,
        incoming_memory_mb,
        config.sandbox_max_memory_mb,
        config.sandbox_host_memory_budget_mb,
        reserved_mb,
    )
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

/// Enforce `SANDBOX_HOST_CPU_BUDGET` at admission. Must be called with
/// [`CREATION_PERMIT`] held so the running-CPU sum cannot race a concurrent
/// create. Mirrors [`enforce_host_memory_budget`].
pub(crate) fn enforce_host_cpu_budget(
    config: &SidecarRuntimeConfig,
    incoming_cpu_cores: u64,
    reused_sandbox_id: Option<&str>,
) -> Result<()> {
    if config.sandbox_host_cpu_budget == 0 {
        return Ok(());
    }

    let running_cpu_cores: Vec<u64> = sandboxes()?
        .values()?
        .into_iter()
        .filter(|record| record.state == SandboxState::Running)
        // A create that replaces an existing record (recreate / image upgrade)
        // frees the old container's CPU, so it doesn't count against the budget.
        .filter(|record| reused_sandbox_id != Some(record.id.as_str()))
        .map(|record| record.cpu_cores)
        .collect();

    check_host_cpu_budget(
        running_cpu_cores,
        incoming_cpu_cores,
        config.sandbox_max_cpu_cores,
        config.sandbox_host_cpu_budget,
    )
}

/// Per-sandbox resource maxima + host memory/CPU budgets, applied under
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
    enforce_host_memory_budget(config, admitted.memory_mb, sandbox_id_override)?;
    enforce_host_cpu_budget(config, admitted.cpu_cores, sandbox_id_override)?;
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
