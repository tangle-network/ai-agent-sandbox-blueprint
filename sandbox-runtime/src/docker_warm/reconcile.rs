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
//! ## The data-loss guard
//!
//! A claimed container keeps the (immutable) `tangle.warm-pool` label after the
//! claim renames it, so the label filter also returns live sandboxes. The guard
//! is `reap iff label present AND container id is NOT a live store record's
//! container_id`. A successfully-claimed container ALWAYS has a durable store
//! record with its id, so it is classified live and left untouched — the Docker
//! analogue of Firecracker's socket-exists live-vs-orphan guard. This guard is
//! stronger than a name check: it also reaps a container that was renamed but
//! crashed before its record was inserted (label present, id not in store),
//! closing that leak window.

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

    let live_ids = live_store_container_ids();
    let to_reap = containers_to_reap(&listings, &live_ids);
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
}

/// Container ids of every live store record (all backends). A warm-labelled
/// container whose id is in this set was claimed and is a live sandbox.
fn live_store_container_ids() -> std::collections::HashSet<String> {
    crate::runtime::sandboxes()
        .and_then(|s| s.values())
        .map(|records| records.into_iter().map(|r| r.container_id).collect())
        .unwrap_or_default()
}

/// Pure reap decision: a warm-labelled container is reaped iff its id is NOT a
/// live store record's container_id. Separated from Docker I/O so the guard is
/// unit-testable without a daemon.
pub(crate) fn containers_to_reap(
    listings: &[WarmContainerListing],
    live_ids: &std::collections::HashSet<String>,
) -> Vec<String> {
    listings
        .iter()
        .filter(|c| !live_ids.contains(&c.id))
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
        .filter_map(|s| s.id.map(|id| WarmContainerListing { id }))
        .collect())
}
