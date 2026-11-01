//! Reap Docker warm containers left by a previous process.
//!
//! A corrupt `sandboxes.json` loads as an empty map, so the store must not
//! decide what is live. Reap only names that still start with `sidecar-warm-`.
//! A claim renames to `sidecar-<id>` before serve. Do not adopt across
//! restart: a stale image would be handed out.

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
