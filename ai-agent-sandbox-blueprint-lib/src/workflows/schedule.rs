use super::*;

pub fn resolve_next_run(
    trigger_type: &str,
    trigger_config: &str,
    last_run_at: Option<u64>,
) -> Result<Option<u64>, String> {
    if trigger_type != "cron" {
        return Ok(None);
    }
    let start = last_run_at.unwrap_or_else(now_ts);
    Ok(Some(compute_next_run(trigger_config, start)?))
}

fn compute_next_run(cron_expr: &str, from_ts: u64) -> Result<u64, String> {
    let schedule =
        Schedule::from_str(cron_expr).map_err(|err| format!("Invalid cron expression: {err}"))?;
    let base = Utc
        .timestamp_opt(from_ts as i64, 0)
        .single()
        .ok_or_else(|| "Invalid timestamp".to_string())?;
    schedule
        .after(&base)
        .next()
        .map(|dt| dt.timestamp().max(0) as u64)
        .ok_or_else(|| "Cron expression has no future run times".to_string())
}
