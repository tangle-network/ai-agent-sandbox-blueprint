//! Restart reconciliation: reap warm containers orphaned by a previous
//! operator process.
//!
//! Warm containers persist across an operator restart, but the fresh process's
//! in-memory pool is empty and the store reconcile
//! ([`crate::reaper::reconcile_on_startup`]) only walks store records — a warm
//! container never entered the store, so without this it orphans invisibly and
//! holds RAM + a host port forever. This lists containers by the
//! `tangle.warm-pool=1` label and reaps every one that is NOT a live sandbox.
//!
//! ## Reap-always (not adopt) — a deliberate v1 choice
//!
//! Unlike the design's adopt-or-reap sketch, this reaps every warm orphan and
//! lets the pool reseed on the current image. That is exactly what the
//! Firecracker pool does (its process-local templates are reaped every
//! restart), it is strictly simpler, and it makes image-staleness a non-issue:
//! a warm container built on a superseded `SIDECAR_IMAGE` is reaped on restart,
//! so the pool never hands out a stale sidecar. Re-adopting the pool across a
//! clean same-image restart (to skip the reseed cost) is a possible follow-up,
//! not v1.
//!
//! ## The data-loss guard (purely structural — never trusts the store)
//!
//! A claimed container keeps the (immutable) `tangle.warm-pool` label after the
//! claim renames it, so the label filter alone ALSO returns live customer
//! sandboxes. Force-removing one would destroy a running customer's work — and
//! this runs at operator startup, right after a crash/redeploy, exactly when the
//! store is most likely partial or unreadable.
//!
//! The guard is therefore purely STRUCTURAL and depends on nothing but the
//! container's own name: a container is a reap candidate only if its name still
//! carries the [`WARM_NAME_PREFIX`] (`sidecar-warm-`). A claim renames the
//! container to `sidecar-<sandbox_id>` BEFORE it is served, so a live claimed
//! sandbox can NEVER match this prefix. Reconcile also only ever runs before the
//! first seed (the in-memory pool is empty), so every `sidecar-warm-` container
//! it sees is necessarily an orphan from a previous process.
//!
//! We deliberately do NOT cross-check container ids against the store to reap
//! renamed orphans. Deriving "not live" from the store is unsafe: a corrupt
//! `sandboxes.json` silently loads as an EMPTY map (`Ok(empty)`, not `Err`), and
//! a poisoned lock / IO error yields `Err` — in every one of those failure modes
//! a live claimed sandbox looks "not in the store" and would be reaped. The only
//! thing that costs is a leaked container from a claim that crashed in the
//! microsecond window between the rename and the store insert; that is a rare,
//! recoverable wasted-RAM leak, never customer data loss. Safety wins.

use super::*;

/// Reap warm containers orphaned by a previous operator process. Runs BEFORE
/// the first seed — from the pool's lazy init closure, and from
/// [`crate::reaper::reconcile_on_startup`] (covering the "warm disabled now, but
/// a prior process left orphans" case the lazy init never reaches).
pub(crate) async fn reconcile_docker_warm_orphans(builder: &DockerBuilder) {
    let listings = match list_warm_containers(builder).await {
        Ok(l) => l,
        Err(err) => {
            tracing::warn!(%err, "docker warm-pool reconcile: list_containers failed");
            return;
        }
    };
    if listings.is_empty() {
        return;
    }

    // Reap decision is purely structural (name prefix) — it never reads the
    // store, so no store failure mode can misclassify a live claimed sandbox as
    // reapable. See the module doc's data-loss guard.
    let to_reap = containers_to_reap(&listings);
    if to_reap.is_empty() {
        return;
    }

    for id in to_reap {
        tracing::warn!(
            container_id = %id,
            "reaping orphaned warm-pool container from a previous operator process"
        );
        let container = match Container::from_id(builder.client(), &id).await {
            Ok(c) => c,
            Err(err) => {
                tracing::warn!(container_id = %id, %err, "warm reconcile: load container failed");
                continue;
            }
        };
        if let Err(err) = container
            .remove(Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            }))
            .await
        {
            tracing::warn!(container_id = %id, %err, "warm reconcile: force-remove failed");
        }
    }
}

/// A warm-labelled container as seen by `list_containers`.
#[derive(Debug, Clone)]
pub(crate) struct WarmContainerListing {
    pub id: String,
    /// Primary container name with the leading `/` stripped (empty if Docker
    /// returned none). `sidecar-warm-<seq>` while pooled; `sidecar-<id>` once
    /// claimed — the structural signal the reap guard keys on.
    pub name: String,
}

/// Pure reap decision (unit-testable without a daemon). A warm-labelled
/// container is reaped iff its name still carries [`WARM_NAME_PREFIX`]
/// (`sidecar-warm-`) — i.e. it is an unclaimed pooled container orphaned by a
/// previous process. A claimed container was renamed to `sidecar-<id>` before
/// being served, so it never matches and is never reaped, regardless of store
/// state. See the module doc for why the store is deliberately not consulted.
pub(crate) fn containers_to_reap(listings: &[WarmContainerListing]) -> Vec<String> {
    listings
        .iter()
        .filter(|c| c.name.starts_with(WARM_NAME_PREFIX))
        .map(|c| c.id.clone())
        .collect()
}

/// List all containers (any state) carrying the `tangle.warm-pool=1` label.
async fn list_warm_containers(builder: &DockerBuilder) -> Result<Vec<WarmContainerListing>> {
    let mut filters = std::collections::HashMap::new();
    filters.insert("label".to_string(), vec![format!("{WARM_POOL_LABEL}=1")]);
    let options = ListContainersOptions {
        all: true,
        filters,
        ..Default::default()
    };
    let summaries = crate::runtime::docker_timeout(
        "warm_list_containers",
        builder.client().list_containers(Some(options)),
    )
    .await?;
    Ok(summaries
        .into_iter()
        .filter_map(|s| {
            s.id.map(|id| {
                // Docker returns names with a leading `/`; the reap guard matches
                // on the bare `sidecar-warm-` prefix.
                let name = s
                    .names
                    .as_ref()
                    .and_then(|n| n.first())
                    .map(|n| n.trim_start_matches('/').to_string())
                    .unwrap_or_default();
                WarmContainerListing { id, name }
            })
        })
        .collect())
}
