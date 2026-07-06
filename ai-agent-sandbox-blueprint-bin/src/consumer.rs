//! Heartbeat consumer + the reconciling Tangle job-result consumer.

use super::*;

/// Logging heartbeat consumer that records heartbeat submissions.
///
/// The actual on-chain submission is handled internally by `HeartbeatService`
/// via ECDSA signing + `submitHeartbeat` contract call. This consumer provides
/// a hook for blueprint-level logging/monitoring of heartbeat events.
#[cfg(feature = "qos")]
#[derive(Clone)]
pub(crate) struct LoggingHeartbeatConsumer;

#[cfg(feature = "qos")]
impl HeartbeatConsumer for LoggingHeartbeatConsumer {
    fn send_heartbeat(
        &self,
        status: &blueprint_qos::heartbeat::HeartbeatStatus,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = blueprint_qos::error::Result<()>> + Send + 'static>,
    > {
        let service_id = status.service_id;
        let status_code = status.status_code;
        let ts = status.timestamp;
        Box::pin(async move {
            info!("Heartbeat sent: service={service_id} status={status_code} ts={ts}");
            Ok(())
        })
    }
}

pub(crate) struct DerivedJobResult {
    service_id: u64,
    call_id: u64,
    output: blueprint_sdk::alloy::primitives::Bytes,
}

pub(crate) enum ConsumerState {
    WaitingForResult,
    ProcessingSubmission(
        Pin<Box<dyn std::future::Future<Output = Result<(), ReconciledConsumerError>> + Send>>,
    ),
}

impl ConsumerState {
    fn is_waiting(&self) -> bool {
        matches!(self, Self::WaitingForResult)
    }
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ReconciledConsumerError {
    #[error("Invalid metadata: {0}")]
    InvalidMetadata(&'static str),
    #[error("Transaction error: {0}")]
    Transaction(String),
}

pub(crate) struct ReconciledTangleConsumer {
    client: Arc<TangleClient>,
    buffer: Mutex<VecDeque<DerivedJobResult>>,
    state: Mutex<ConsumerState>,
}

impl ReconciledTangleConsumer {
    pub(crate) fn new(client: TangleClient) -> Self {
        Self {
            client: Arc::new(client),
            buffer: Mutex::new(VecDeque::new()),
            state: Mutex::new(ConsumerState::WaitingForResult),
        }
    }
}

impl Sink<blueprint_sdk::JobResult> for ReconciledTangleConsumer {
    type Error = BoxError;

    fn poll_ready(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, item: blueprint_sdk::JobResult) -> Result<(), Self::Error> {
        let blueprint_sdk::JobResult::Ok { head, body } = &item else {
            blueprint_sdk::trace!(target: "tangle-consumer", "Discarding job result with error");
            return Ok(());
        };

        let (Some(call_id_raw), Some(service_id_raw)) = (
            head.metadata.get(CallId::METADATA_KEY),
            head.metadata.get(ServiceId::METADATA_KEY),
        ) else {
            blueprint_sdk::trace!(
                target: "tangle-consumer",
                "Discarding job result with missing metadata"
            );
            return Ok(());
        };

        let call_id: u64 = call_id_raw
            .try_into()
            .map_err(|_| ReconciledConsumerError::InvalidMetadata("call_id"))?;
        let service_id: u64 = service_id_raw
            .try_into()
            .map_err(|_| ReconciledConsumerError::InvalidMetadata("service_id"))?;

        self.get_mut()
            .buffer
            .lock()
            .unwrap()
            .push_back(DerivedJobResult {
                service_id,
                call_id,
                output: blueprint_sdk::alloy::primitives::Bytes::copy_from_slice(body),
            });
        Ok(())
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let consumer = self.get_mut();
        let mut state = consumer.state.lock().unwrap();

        {
            let buffer = consumer.buffer.lock().unwrap();
            if buffer.is_empty() && state.is_waiting() {
                return Poll::Ready(Ok(()));
            }
        }

        loop {
            match &mut *state {
                ConsumerState::WaitingForResult => {
                    let next = {
                        let mut buffer = consumer.buffer.lock().unwrap();
                        buffer.pop_front()
                    };

                    let Some(DerivedJobResult {
                        service_id,
                        call_id,
                        output,
                    }) = next
                    else {
                        return Poll::Ready(Ok(()));
                    };

                    let client = Arc::clone(&consumer.client);
                    let fut = Box::pin(async move {
                        submit_result_and_reconcile(client, service_id, call_id, output).await
                    });
                    *state = ConsumerState::ProcessingSubmission(fut);
                }
                ConsumerState::ProcessingSubmission(future) => match future.as_mut().poll(cx) {
                    Poll::Ready(Ok(())) => {
                        *state = ConsumerState::WaitingForResult;
                    }
                    Poll::Ready(Err(err)) => {
                        *state = ConsumerState::WaitingForResult;
                        return Poll::Ready(Err(err.into()));
                    }
                    Poll::Pending => return Poll::Pending,
                },
            }
        }
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let buffer = self.buffer.lock().unwrap();
        if buffer.is_empty() {
            Poll::Ready(Ok(()))
        } else {
            Poll::Pending
        }
    }
}

pub(crate) async fn submit_result_and_reconcile(
    client: Arc<TangleClient>,
    service_id: u64,
    call_id: u64,
    output: blueprint_sdk::alloy::primitives::Bytes,
) -> Result<(), ReconciledConsumerError> {
    if client.config.dry_run {
        info!(
            "Dry run enabled; skipping on-chain result submission for service {service_id} call {call_id}"
        );
        return Ok(());
    }

    match client
        .submit_result(service_id, call_id, output.clone())
        .await
    {
        Ok(result) if result.success => {
            reconcile_workflows(&client, service_id).await;
            Ok(())
        }
        Ok(result) => Err(ReconciledConsumerError::Transaction(format!(
            "Transaction reverted for service {service_id} call {call_id}: tx_hash={:?}",
            result.tx_hash
        ))),
        Err(err) if is_job_already_completed(&err.to_string()) => {
            warn!(
                "Result for service {service_id} call {call_id} was already completed; treating replay as idempotent"
            );
            reconcile_workflows(&client, service_id).await;
            Ok(())
        }
        Err(err)
            if replay_error_is_already_materialized(
                &client,
                service_id,
                call_id,
                &output,
                &err.to_string(),
            )
            .await =>
        {
            warn!(
                "Result for service {service_id} call {call_id} is already reflected on-chain; treating replay as idempotent"
            );
            reconcile_workflows(&client, service_id).await;
            Ok(())
        }
        Err(err) => Err(ReconciledConsumerError::Transaction(format!(
            "Failed to submit result for service {service_id} call {call_id}: {err}"
        ))),
    }
}

pub(crate) async fn reconcile_workflows(client: &TangleClient, service_id: u64) {
    if let Err(err) = bootstrap_workflows_from_chain(client, service_id).await {
        warn!("Failed to reconcile workflows from chain for service {service_id}: {err}");
    }
}

pub(crate) fn is_job_already_completed(error: &str) -> bool {
    error.contains("JobAlreadyCompleted") || error.contains("already completed")
}

pub(crate) async fn replay_error_is_already_materialized(
    client: &TangleClient,
    service_id: u64,
    call_id: u64,
    output: &blueprint_sdk::alloy::primitives::Bytes,
    error: &str,
) -> bool {
    if !error.contains("execution reverted") {
        return false;
    }

    if bootstrap_workflows_from_chain(client, service_id)
        .await
        .is_err()
    {
        return false;
    }

    let workflow_for_call_id = workflows()
        .ok()
        .and_then(|store| store.get(&workflow_key(call_id)).ok())
        .flatten();

    let payload = JsonResponse::abi_decode(output.as_ref())
        .ok()
        .and_then(|response| serde_json::from_str::<Value>(&response.json).ok());

    if let Some(payload) = payload.as_ref() {
        let Some(workflow_id) = payload.get("workflowId").and_then(Value::as_u64) else {
            return false;
        };

        let workflow = workflows()
            .ok()
            .and_then(|store| store.get(&workflow_key(workflow_id)).ok())
            .flatten();

        if workflow_replay_matches_store(call_id, payload, workflow.as_ref()) {
            return true;
        }
    }

    if let Ok(create_output) = SandboxCreateOutput::abi_decode(output.as_ref())
        && ai_agent_sandbox_blueprint_lib::runtime::get_sandbox_by_id(&create_output.sandboxId)
            .is_ok()
    {
        return true;
    }

    // Workflow IDs are derived from the create call ID. If a replayed
    // `workflow_create` result arrives before we can decode its body cleanly,
    // an active workflow keyed by the same call ID is still enough evidence
    // that the original result has already been materialized on-chain.
    workflow_for_call_id
        .as_ref()
        .is_some_and(|entry| entry.active)
}

pub(crate) fn workflow_replay_matches_store(
    call_id: u64,
    payload: &Value,
    workflow: Option<&WorkflowEntry>,
) -> bool {
    let Some(workflow_id) = payload.get("workflowId").and_then(Value::as_u64) else {
        return false;
    };

    match payload.get("status").and_then(Value::as_str) {
        Some("canceled") => workflow.is_none(),
        Some("active") => workflow.as_ref().is_some(),
        _ if payload.get("task").is_some() => workflow.as_ref().is_some(),
        _ => workflow_id == call_id && workflow.as_ref().is_some(),
    }
}
