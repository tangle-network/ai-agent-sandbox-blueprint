use super::*;

/// Read service config from the BSM contract via RPC.
///
/// Returns the raw config bytes, or `None` if no config is stored yet.
pub async fn read_service_config(config: &AutoProvisionConfig) -> Result<Option<Vec<u8>>, String> {
    let url: url::Url = config
        .http_rpc_endpoint
        .parse()
        .map_err(|e| format!("Invalid RPC URL: {e}"))?;

    let provider = ProviderBuilder::new().connect_http(url);
    let contract = IBsmRead::new(config.bsm_address, &provider);

    let result = contract
        .getServiceConfig(config.service_id)
        .call()
        .await
        .map_err(|e| format!("getServiceConfig RPC failed: {e}"))?;

    let bytes = result.0;
    if bytes.is_empty() {
        Ok(None)
    } else {
        Ok(Some(bytes.to_vec()))
    }
}

/// Read service owner from the BSM contract via RPC.
///
/// Returns the owner address as a lowercase hex string, or empty string if not set.
pub async fn read_service_owner(config: &AutoProvisionConfig) -> Result<String, String> {
    let url: url::Url = config
        .http_rpc_endpoint
        .parse()
        .map_err(|e| format!("Invalid RPC URL: {e}"))?;

    let provider = ProviderBuilder::new().connect_http(url);
    let contract = IBsmRead::new(config.bsm_address, &provider);

    let result = contract
        .serviceOwner(config.service_id)
        .call()
        .await
        .map_err(|e| format!("serviceOwner RPC failed: {e}"))?;

    let addr = result.0;
    if addr.is_zero() {
        Ok(String::new())
    } else {
        Ok(format!("{addr}").to_lowercase())
    }
}

/// Decode raw config bytes as a `ProvisionRequest`.
///
/// The on-chain config is stored as ABI-encoded params (flat tuple, no outer offset prefix),
/// e.g. from `cast abi-encode "f(string,...)" ...` or `abi.encode(field1, field2, ...)`.
/// Accept both params-encoded and tuple-encoded representations.
pub fn decode_provision_config(config_bytes: &[u8]) -> Result<ProvisionRequest, String> {
    LegacyProvisionRequest::abi_decode_params(config_bytes)
        .map(ProvisionRequest::from)
        .or_else(|_| LegacyProvisionRequest::abi_decode(config_bytes).map(ProvisionRequest::from))
        .or_else(|_| ProvisionRequest::abi_decode_params(config_bytes))
        .or_else(|_| ProvisionRequest::abi_decode(config_bytes))
        .or_else(|_| {
            ProvisionRequestV1::abi_decode_params(config_bytes).map(ProvisionRequest::from)
        })
        .or_else(|_| ProvisionRequestV1::abi_decode(config_bytes).map(ProvisionRequest::from))
        .map_err(|e| format!("Failed to decode ProvisionRequest from service config: {e}"))
}
