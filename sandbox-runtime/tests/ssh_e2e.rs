use std::fs;
use std::process::Command;
use std::time::Duration;

use tempfile::TempDir;
use tokio::sync::Mutex;

use sandbox_runtime::runtime::{
    CreateSandboxParams, create_sidecar, delete_sidecar, provision_ssh_key,
};

// These tests mutate process env and rely on global OnceCell state.
static TEST_LOCK: Mutex<()> = Mutex::const_new(());
const TEST_IMAGE: &str = "agent-dev:latest";

fn setup_env(state_dir: &TempDir) {
    unsafe {
        std::env::set_var("BLUEPRINT_STATE_DIR", state_dir.path());
        std::env::set_var("SIDECAR_IMAGE", TEST_IMAGE);
        std::env::set_var("SIDECAR_PULL_IMAGE", "false");
        std::env::set_var("SIDECAR_PUBLIC_HOST", "127.0.0.1");
        std::env::set_var("REQUEST_TIMEOUT_SECS", "60");
        std::env::set_var("SESSION_AUTH_SECRET", "ssh-e2e-test-secret");
        std::env::remove_var("DOCKER_HOST");
    }
}

fn command_present(program: &str) -> bool {
    Command::new("sh")
        .args(["-lc", &format!("command -v {program} >/dev/null 2>&1")])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn image_exists(image: &str) -> bool {
    Command::new("docker")
        .args(["image", "inspect", image])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

fn image_supports_compatible_ssh_user(image: &str) -> bool {
    Command::new("docker")
        .args([
            "run",
            "--rm",
            "--entrypoint",
            "sh",
            image,
            "-lc",
            "getent passwd sidecar >/dev/null 2>&1 || getent passwd agent >/dev/null 2>&1",
        ])
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn generate_test_key(dir: &TempDir) -> (String, String) {
    let key_path = dir.path().join("id_ed25519");
    let status = Command::new("ssh-keygen")
        .args(["-t", "ed25519", "-f"])
        .arg(&key_path)
        .args(["-N", "", "-q"])
        .status()
        .expect("ssh-keygen should run");
    assert!(status.success(), "ssh-keygen failed: {status}");

    let private_key = key_path.to_string_lossy().into_owned();
    let public_key = fs::read_to_string(key_path.with_extension("pub"))
        .expect("public key should be readable")
        .trim()
        .to_string();
    (private_key, public_key)
}

fn ssh_command(private_key: &str, username: &str, port: u16, remote: Option<&str>) -> Command {
    let mut cmd = Command::new("ssh");
    cmd.arg("-i")
        .arg(private_key)
        .args([
            "-o",
            "StrictHostKeyChecking=no",
            "-o",
            "UserKnownHostsFile=/dev/null",
            "-o",
            "ConnectTimeout=10",
            "-p",
        ])
        .arg(port.to_string())
        .arg(format!("{username}@127.0.0.1"));
    if let Some(remote_cmd) = remote {
        cmd.arg(remote_cmd);
    }
    cmd
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn docker_ssh_supports_commands_and_interactive_shell() {
    let _guard = TEST_LOCK.lock().await;
    if !command_present("docker")
        || !Command::new("docker")
            .args(["info"])
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
    {
        eprintln!("SKIP: Docker is not available");
        return;
    }
    if !command_present("ssh") {
        eprintln!("SKIP: ssh client is not available");
        return;
    }
    if !command_present("ssh-keygen") {
        eprintln!("SKIP: ssh-keygen is not available");
        return;
    }
    if !image_exists(TEST_IMAGE) {
        eprintln!("SKIP: required Docker image {TEST_IMAGE} is not available locally");
        return;
    }
    if !image_supports_compatible_ssh_user(TEST_IMAGE) {
        eprintln!("SKIP: Docker image {TEST_IMAGE} does not provide a compatible SSH user");
        return;
    }

    let state_dir = TempDir::new().expect("temp state dir");
    let key_dir = TempDir::new().expect("temp key dir");
    setup_env(&state_dir);
    let (private_key, public_key) = generate_test_key(&key_dir);

    let params = CreateSandboxParams {
        name: "ssh-e2e".to_string(),
        image: TEST_IMAGE.to_string(),
        stack: "default".to_string(),
        agent_identifier: "default".to_string(),
        env_json: String::new(),
        metadata_json: r#"{"runtime_backend":"docker"}"#.to_string(),
        ssh_enabled: true,
        ssh_public_key: String::new(),
        web_terminal_enabled: false,
        max_lifetime_seconds: 3600,
        idle_timeout_seconds: 3600,
        cpu_cores: 2,
        memory_mb: 2048,
        disk_gb: 10,
        owner: "0x9965507d1a55bcc2695c58ba16fb37d819b0a4dc".to_string(),
        service_id: None,
        tee_config: None,
        user_env_json: String::new(),
        port_mappings: Vec::new(),
        capabilities_json: String::new(),
    };

    let (record, _) = create_sidecar(&params, None)
        .await
        .expect("sandbox should be created");
    let cleanup_record = record.clone();

    let test_result = async {
        let port = record.ssh_port.expect("ssh port should be exposed");
        let login_user = record
            .ssh_login_user
            .clone()
            .expect("ssh login user should be detected during creation");
        let (username, _) = provision_ssh_key(&record, None, &public_key)
            .await
            .expect("ssh key should provision");
        assert_eq!(username, login_user);

        tokio::time::sleep(Duration::from_secs(1)).await;

        let command_output = ssh_command(
            &private_key,
            &login_user,
            port,
            Some("whoami && pwd && echo SSH works!"),
        )
        .output()
        .expect("ssh command mode should run");
        assert!(
            command_output.status.success(),
            "ssh command mode failed: stdout={} stderr={}",
            String::from_utf8_lossy(&command_output.stdout),
            String::from_utf8_lossy(&command_output.stderr)
        );

        let command_stdout = String::from_utf8_lossy(&command_output.stdout);
        assert!(command_stdout.contains(&login_user), "stdout={command_stdout}");
        assert!(
            command_stdout.contains(&format!("/home/{login_user}")),
            "stdout={command_stdout}"
        );
        assert!(command_stdout.contains("SSH works!"), "stdout={command_stdout}");

        let interactive = Command::new("sh")
            .arg("-lc")
            .arg(format!(
                "printf 'whoami\\npwd\\nexit\\n' | ssh -tt -i '{private_key}' -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=10 -p {port} {login_user}@127.0.0.1"
            ))
            .output()
            .expect("interactive ssh should run");

        assert!(
            interactive.status.success(),
            "interactive ssh failed: stdout={} stderr={}",
            String::from_utf8_lossy(&interactive.stdout),
            String::from_utf8_lossy(&interactive.stderr)
        );

        let interactive_stdout = String::from_utf8_lossy(&interactive.stdout);
        let interactive_stderr = String::from_utf8_lossy(&interactive.stderr);
        let interactive_text = format!("{interactive_stdout}\n{interactive_stderr}");
        assert!(
            interactive_text.contains(&login_user),
            "interactive output={interactive_text}"
        );
        assert!(
            interactive_text.contains(&format!("/home/{login_user}")),
            "interactive output={interactive_text}"
        );
    }
    .await;

    let _ = delete_sidecar(&cleanup_record, None).await;
    test_result
}
