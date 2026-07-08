use super::*;

#[derive(Debug, Default, Clone)]
pub(crate) struct ExecCommandResult {
    pub(crate) exit_code: i64,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

pub(crate) fn exec_result_json(result: &ExecCommandResult) -> Value {
    json!({
        "result": {
            "exitCode": result.exit_code,
            "stdout": result.stdout,
            "stderr": result.stderr,
        }
    })
}

pub(crate) fn summarize_exec_failure(result: &ExecCommandResult) -> String {
    result
        .stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .or_else(|| {
            result
                .stdout
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
        })
        .unwrap_or("command failed")
        .to_string()
}

pub(crate) fn parse_sidecar_exec_result(parsed: &Value) -> ExecCommandResult {
    let result = parsed.get("result");
    ExecCommandResult {
        exit_code: result
            .and_then(|r| r.get("exitCode"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
        stdout: result
            .and_then(|r| r.get("stdout"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        stderr: result
            .and_then(|r| r.get("stderr"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}

pub(crate) fn extract_detected_ssh_username(result: &ExecCommandResult) -> Result<String> {
    if result.exit_code != 0 {
        return Err(SandboxError::Validation(format!(
            "SSH username detection failed (exit {}): {}",
            result.exit_code,
            summarize_exec_failure(result)
        )));
    }

    for line in result.stdout.lines() {
        let candidate = line.trim();
        if candidate.is_empty() {
            continue;
        }
        if crate::ssh_validation::validate_ssh_username(candidate).is_ok() {
            return Ok(candidate.to_string());
        }
    }

    Err(SandboxError::Validation(
        "SSH username detection failed: could not find a valid username in command output".into(),
    ))
}

pub(crate) async fn docker_exec_as_user(
    container_id: &str,
    user: &str,
    command: &str,
) -> Result<ExecCommandResult> {
    let builder = docker_builder().await?;
    let exec = docker_timeout(
        "create_exec",
        builder.client().create_exec(
            container_id,
            CreateExecOptions::<String> {
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                cmd: Some(vec![
                    "/bin/sh".to_string(),
                    "-lc".to_string(),
                    command.to_string(),
                ]),
                user: Some(user.to_string()),
                ..Default::default()
            },
        ),
    )
    .await?;

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    match docker_timeout(
        "start_exec",
        builder
            .client()
            .start_exec(&exec.id, None::<StartExecOptions>),
    )
    .await?
    {
        StartExecResults::Attached { mut output, .. } => {
            while let Some(chunk) = output.next().await {
                let chunk =
                    chunk.map_err(|e| SandboxError::Docker(format!("exec output failed: {e}")))?;
                match chunk {
                    LogOutput::StdOut { message } | LogOutput::Console { message } => {
                        stdout.extend_from_slice(&message);
                    }
                    LogOutput::StdErr { message } => stderr.extend_from_slice(&message),
                    LogOutput::StdIn { .. } => {}
                }
            }
        }
        StartExecResults::Detached => {
            return Err(SandboxError::Docker(
                "exec unexpectedly detached while waiting for SSH bootstrap output".into(),
            ));
        }
    }

    let inspect = docker_timeout("inspect_exec", builder.client().inspect_exec(&exec.id)).await?;
    Ok(ExecCommandResult {
        exit_code: inspect.exit_code.unwrap_or_default(),
        stdout: String::from_utf8_lossy(&stdout).into_owned(),
        stderr: String::from_utf8_lossy(&stderr).into_owned(),
    })
}

pub(crate) fn normalize_requested_ssh_username(username: Option<&str>) -> Result<Option<String>> {
    let Some(username) = username.map(str::trim) else {
        return Ok(None);
    };
    if username.is_empty() {
        return Ok(None);
    }
    crate::ssh_validation::validate_ssh_username(username).map_err(SandboxError::Validation)?;
    Ok(Some(username.to_string()))
}

pub(crate) fn persist_ssh_login_user(sandbox_id: &str, username: &str) -> Result<()> {
    sandboxes()?.update(sandbox_id, |record| {
        record.ssh_login_user = Some(username.to_string());
    })?;
    Ok(())
}

pub(crate) fn persist_ssh_key_assignment(
    sandbox_id: &str,
    username: &str,
    public_key: &str,
) -> Result<()> {
    sandboxes()?.update(sandbox_id, |record| {
        let entry = SshAuthorizedKey {
            username: username.to_string(),
            public_key: public_key.to_string(),
        };
        if !record.ssh_authorized_keys.contains(&entry) {
            record.ssh_authorized_keys.push(entry);
        }
    })?;
    Ok(())
}

pub(crate) fn remove_ssh_key_assignment(
    sandbox_id: &str,
    username: &str,
    public_key: &str,
) -> Result<()> {
    sandboxes()?.update(sandbox_id, |record| {
        record
            .ssh_authorized_keys
            .retain(|entry| !(entry.username == username && entry.public_key == public_key));
    })?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn select_docker_ssh_login_user<'a, F>(mut user_exists: F) -> Option<&'a str>
where
    F: FnMut(&str) -> bool,
{
    SSH_COMPATIBLE_LOGIN_USERS
        .iter()
        .copied()
        .find(|candidate| user_exists(candidate))
}

pub(crate) fn compatible_docker_ssh_users_summary() -> String {
    SSH_COMPATIBLE_LOGIN_USERS.join(", ")
}

pub(crate) async fn docker_user_exists(container_id: &str, username: &str) -> Result<bool> {
    let user_arg = shell_escape(username);
    let command = format!("getent passwd {user_arg} >/dev/null 2>&1");
    let result = docker_exec_as_user(container_id, "root", &command).await?;
    Ok(result.exit_code == 0)
}

pub(crate) async fn detect_docker_ssh_username(record: &SandboxRecord) -> Result<String> {
    if let Some(username) = &record.ssh_login_user {
        return Ok(username.clone());
    }

    for candidate in SSH_COMPATIBLE_LOGIN_USERS {
        if docker_user_exists(&record.container_id, candidate).await? {
            persist_ssh_login_user(&record.id, candidate)?;
            return Ok((*candidate).to_string());
        }
    }

    Err(SandboxError::Validation(format!(
        "SSH login user detection failed for sandbox {}: none of the supported users exist (checked: {})",
        record.id,
        compatible_docker_ssh_users_summary()
    )))
}

pub(crate) fn resolve_docker_ssh_username(
    record: &SandboxRecord,
    requested: Option<String>,
) -> Result<String> {
    let login_user = record
        .ssh_login_user
        .clone()
        .unwrap_or_else(|| SSH_DEFAULT_LOGIN_USER.to_string());
    match requested {
        Some(username) if username != login_user => Err(SandboxError::Validation(format!(
            "SSH login is only supported for user '{login_user}'"
        ))),
        Some(username) => Ok(username),
        None => Ok(login_user),
    }
}

pub(crate) async fn ensure_docker_ssh_ready(record: &SandboxRecord) -> Result<String> {
    let login_user = detect_docker_ssh_username(record).await?;
    let root_bootstrap = docker_exec_as_user(
        &record.container_id,
        "root",
        &build_docker_ssh_bootstrap_command(&login_user),
    )
    .await?;
    if root_bootstrap.exit_code != 0 {
        return Err(SandboxError::Validation(format!(
            "SSH bootstrap failed for sandbox {}: {}",
            record.id,
            summarize_exec_failure(&root_bootstrap)
        )));
    }

    let home_bootstrap = docker_exec_as_user(
        &record.container_id,
        &login_user,
        &build_docker_ssh_user_home_bootstrap_command(&login_user),
    )
    .await?;
    if home_bootstrap.exit_code != 0 {
        return Err(SandboxError::Validation(format!(
            "SSH bootstrap failed for sandbox {}: {}",
            record.id,
            summarize_exec_failure(&home_bootstrap)
        )));
    }

    persist_ssh_login_user(&record.id, &login_user)?;
    Ok(login_user)
}

pub(crate) fn is_docker_unavailable(err: &SandboxError) -> bool {
    matches!(err, SandboxError::Docker(msg) if msg.contains("Failed to connect to Docker") || msg.contains("Socket not found"))
}

pub(crate) async fn detect_sidecar_ssh_username(record: &SandboxRecord) -> Result<String> {
    let payload = json!({ "command": "id -un || whoami" });
    let parsed = crate::http::sidecar_post_json(
        &record.sidecar_url,
        "/terminals/commands",
        &record.token,
        payload,
    )
    .await?;
    let username = extract_detected_ssh_username(&parse_sidecar_exec_result(&parsed))?;
    persist_ssh_login_user(&record.id, &username)?;
    Ok(username)
}

pub(crate) async fn execute_docker_ssh_command(
    record: &SandboxRecord,
    user: &str,
    command: &str,
) -> Result<ExecCommandResult> {
    let result = docker_exec_as_user(&record.container_id, user, command).await?;
    if result.exit_code != 0 {
        return Err(SandboxError::Validation(format!(
            "SSH command failed for sandbox {} (user {}): {}",
            record.id,
            user,
            summarize_exec_failure(&result)
        )));
    }
    Ok(result)
}

pub(crate) async fn execute_sidecar_ssh_command(
    record: &SandboxRecord,
    command: &str,
) -> Result<Value> {
    let payload = json!({ "command": format!("sh -c {}", shell_escape(command)) });
    crate::http::sidecar_post_json(
        &record.sidecar_url,
        "/terminals/commands",
        &record.token,
        payload,
    )
    .await
}

pub(crate) async fn prepare_ssh_access(record: &SandboxRecord) -> Result<(SandboxRecord, bool)> {
    if record.ssh_port.is_none() {
        return Err(SandboxError::Validation(
            "SSH is not enabled for this sandbox".into(),
        ));
    }

    if supports_docker_endpoint_refresh(record) {
        match ensure_docker_ssh_ready(record).await {
            Ok(_) => return Ok((get_sandbox_by_id(&record.id)?, true)),
            Err(err) if is_docker_unavailable(&err) => {
                return Ok((get_sandbox_by_id(&record.id)?, false));
            }
            Err(err) => return Err(err),
        }
    }

    Ok((get_sandbox_by_id(&record.id)?, false))
}

pub async fn ensure_ssh_ready(record: &SandboxRecord) -> Result<SandboxRecord> {
    let (record, _) = prepare_ssh_access(record).await?;
    Ok(record)
}

pub async fn detect_ssh_username(record: &SandboxRecord) -> Result<String> {
    let (record, docker_managed) = prepare_ssh_access(record).await?;
    if docker_managed {
        return Ok(record
            .ssh_login_user
            .unwrap_or_else(|| SSH_DEFAULT_LOGIN_USER.to_string()));
    }
    if let Some(username) = &record.ssh_login_user {
        return Ok(username.clone());
    }
    detect_sidecar_ssh_username(&record).await
}

pub async fn provision_ssh_key(
    record: &SandboxRecord,
    requested_username: Option<&str>,
    public_key: &str,
) -> Result<(String, Value)> {
    crate::ssh_validation::validate_ssh_public_key(public_key).map_err(SandboxError::Validation)?;
    let requested = normalize_requested_ssh_username(requested_username)?;
    let (ready_record, docker_managed) = prepare_ssh_access(record).await?;
    let username = if docker_managed {
        resolve_docker_ssh_username(&ready_record, requested)?
    } else {
        match requested {
            Some(username) => username,
            None => detect_ssh_username(&ready_record).await?,
        }
    };

    let result_json = if docker_managed {
        exec_result_json(
            &execute_docker_ssh_command(
                &ready_record,
                &username,
                &build_ssh_key_install_command(&username, public_key),
            )
            .await?,
        )
    } else {
        let parsed = execute_sidecar_ssh_command(
            &ready_record,
            &build_sidecar_ssh_key_install_command(&username, public_key),
        )
        .await?;
        let exec = parse_sidecar_exec_result(&parsed);
        if exec.exit_code != 0 {
            return Err(SandboxError::Validation(format!(
                "SSH provision failed for user '{username}' (exit {}): {}",
                exec.exit_code,
                summarize_exec_failure(&exec)
            )));
        }
        parsed
    };

    persist_ssh_login_user(&ready_record.id, &username)?;
    persist_ssh_key_assignment(&ready_record.id, &username, public_key)?;
    Ok((username, result_json))
}

pub async fn revoke_ssh_key(
    record: &SandboxRecord,
    requested_username: Option<&str>,
    public_key: &str,
) -> Result<(String, Value)> {
    crate::ssh_validation::validate_ssh_public_key(public_key).map_err(SandboxError::Validation)?;
    let requested = normalize_requested_ssh_username(requested_username)?;
    let (ready_record, docker_managed) = prepare_ssh_access(record).await?;
    let username = if docker_managed {
        resolve_docker_ssh_username(&ready_record, requested)?
    } else {
        match requested {
            Some(username) => username,
            None => detect_ssh_username(&ready_record).await?,
        }
    };

    let result_json = if docker_managed {
        exec_result_json(
            &execute_docker_ssh_command(
                &ready_record,
                &username,
                &build_ssh_key_revoke_command(&username, public_key),
            )
            .await?,
        )
    } else {
        let parsed = execute_sidecar_ssh_command(
            &ready_record,
            &build_sidecar_ssh_key_revoke_command(&username, public_key),
        )
        .await?;
        let exec = parse_sidecar_exec_result(&parsed);
        if exec.exit_code != 0 {
            return Err(SandboxError::Validation(format!(
                "SSH revoke failed for user '{username}' (exit {}): {}",
                exec.exit_code,
                summarize_exec_failure(&exec)
            )));
        }
        parsed
    };

    persist_ssh_login_user(&ready_record.id, &username)?;
    remove_ssh_key_assignment(&ready_record.id, &username, public_key)?;
    Ok((username, result_json))
}

pub async fn restore_ssh_access(record: &SandboxRecord) -> Result<SandboxRecord> {
    let (updated, docker_managed) = prepare_ssh_access(record).await?;
    if docker_managed {
        for entry in updated.ssh_authorized_keys.clone() {
            let _ = execute_docker_ssh_command(
                &updated,
                &entry.username,
                &build_ssh_key_install_command(&entry.username, &entry.public_key),
            )
            .await?;
        }
    }
    get_sandbox_by_id(&record.id)
}
