use super::*;

pub(crate) fn build_docker_ssh_bootstrap_command(username: &str) -> String {
    let user_arg = shell_escape(username);
    format!(
        r#"set -eu;
user={user_arg};
shell="/bin/sh";
[ -x "$shell" ] || shell="/bin/bash";
if ! getent passwd "$user" >/dev/null 2>&1; then
  echo "User $user does not exist" >&2;
  exit 1;
fi;
home=$(getent passwd "$user" | cut -d: -f6);
if [ -z "$home" ]; then
  echo "User $user does not have a home directory" >&2;
  exit 1;
fi;
current_shell=$(getent passwd "$user" | cut -d: -f7);
if [ "$current_shell" = "/sbin/nologin" ] || [ "$current_shell" = "/bin/false" ]; then
  awk -F: -v user="$user" -v shell="$shell" 'BEGIN {{ OFS=FS }} $1==user {{ $7=shell }} {{ print }}' /etc/passwd > /tmp/passwd.tangle;
  cat /tmp/passwd.tangle > /etc/passwd;
  rm -f /tmp/passwd.tangle;
fi;
if command -v passwd >/dev/null 2>&1; then
  # OpenSSH rejects locked accounts before checking authorized_keys.
  # `passwd -u` only unlocks an account that *has* a stored password — for
  # the common case of a service user created with `useradd` (no password
  # ever set), `passwd -u` fails with "unlocking the password would result
  # in a passwordless account" and the shadow entry stays `!`-prefixed.
  # `passwd -d` removes the password entirely and clears the lock flag
  # (`NP` in `passwd -S`), which is what we actually want: key-only login,
  # no password fallback. Try -d first; fall through to -u for the
  # platforms that support one but not the other.
  passwd -d "$user" >/dev/null 2>&1 || passwd -u "$user" >/dev/null 2>&1 || true;
fi;
if ! command -v sshd >/dev/null 2>&1; then
  if command -v apk >/dev/null 2>&1; then
    apk add --no-cache openssh-server >/dev/null;
  elif command -v apt-get >/dev/null 2>&1; then
    export DEBIAN_FRONTEND=noninteractive;
    # apt-get install can emit benign non-zero exits when its partial-cache
    # cleanup hits a permission mismatch (e.g. _apt-owned files inside a
    # rootless or user-namespace-remapped runtime). The package itself
    # still gets installed — apt's cleanup is best-effort. We capture the
    # exit and source-of-truth on `command -v sshd` afterwards instead of
    # letting `set -e` abort the bootstrap on a cleanup-only failure.
    apt-get update >/dev/null 2>&1 || true;
    apt-get install -y --no-install-recommends openssh-server >/dev/null 2>&1 || true;
    rm -rf /var/lib/apt/lists/* 2>/dev/null || true;
    if ! command -v sshd >/dev/null 2>&1; then
      echo "openssh-server install failed: sshd binary missing after apt-get install" >&2;
      exit 1;
    fi;
  else
    echo "Unsupported package manager for SSH bootstrap" >&2;
    exit 1;
  fi;
fi;
mkdir -p /run/sshd;
ssh-keygen -A >/dev/null 2>&1;
cat > /etc/ssh/sshd_config.tangle <<'EOF'
Port 22
Protocol 2
HostKey /etc/ssh/ssh_host_rsa_key
HostKey /etc/ssh/ssh_host_ed25519_key
PubkeyAuthentication yes
PasswordAuthentication no
KbdInteractiveAuthentication no
ChallengeResponseAuthentication no
PermitRootLogin no
AllowUsers {username}
AuthorizedKeysFile .ssh/authorized_keys
PidFile /run/sshd.pid
Subsystem sftp internal-sftp
EOF
if ! awk 'NR > 1 {{ split($2,a,":"); if (toupper(a[2]) == "0016" && $4 == "0A") found=1 }} END {{ exit(found ? 0 : 1) }}' /proc/net/tcp /proc/net/tcp6 2>/dev/null; then
  if [ -f /run/sshd.pid ] && kill -0 "$(cat /run/sshd.pid)" 2>/dev/null; then
    kill "$(cat /run/sshd.pid)" 2>/dev/null || true;
    sleep 1;
  fi;
  rm -f /run/sshd.pid;
  /usr/sbin/sshd -f /etc/ssh/sshd_config.tangle;
fi;
awk 'NR > 1 {{ split($2,a,":"); if (toupper(a[2]) == "0016" && $4 == "0A") found=1 }} END {{ exit(found ? 0 : 1) }}' /proc/net/tcp /proc/net/tcp6 2>/dev/null"#,
    )
}

pub(crate) fn build_docker_ssh_user_home_bootstrap_command(username: &str) -> String {
    let user_arg = shell_escape(username);
    format!(
        r#"set -eu;
user={user_arg};
home=$(getent passwd "$user" | cut -d: -f6);
if [ -z "$home" ]; then
  echo "User $user does not exist" >&2;
  exit 1;
fi;
mkdir -p "$home/.ssh";
touch "$home/.ssh/authorized_keys";
chmod 700 "$home/.ssh";
chmod 600 "$home/.ssh/authorized_keys""#
    )
}

pub(crate) fn build_ssh_key_install_command(username: &str, public_key: &str) -> String {
    let user_arg = shell_escape(username);
    let key_arg = shell_escape(public_key);
    format!(
        r#"set -eu;
user={user_arg};
key={key_arg};
home=$(getent passwd "$user" | cut -d: -f6);
if [ -z "$home" ]; then
  echo "User $user does not exist" >&2;
  exit 1;
fi;
mkdir -p "$home/.ssh";
touch "$home/.ssh/authorized_keys";
chmod 700 "$home/.ssh";
if ! grep -qxF "$key" "$home/.ssh/authorized_keys" 2>/dev/null; then
  printf '%s\n' "$key" >> "$home/.ssh/authorized_keys";
fi;
chmod 600 "$home/.ssh/authorized_keys""#
    )
}

pub(crate) fn build_ssh_key_revoke_command(username: &str, public_key: &str) -> String {
    let user_arg = shell_escape(username);
    let key_arg = shell_escape(public_key);
    format!(
        r#"set -eu;
user={user_arg};
key={key_arg};
home=$(getent passwd "$user" | cut -d: -f6);
if [ -z "$home" ]; then
  echo "User $user does not exist" >&2;
  exit 1;
fi;
if [ -f "$home/.ssh/authorized_keys" ]; then
  tmp=$(mktemp /tmp/authorized_keys.XXXXXX);
  grep -vxF "$key" "$home/.ssh/authorized_keys" > "$tmp" || true;
  mv "$tmp" "$home/.ssh/authorized_keys";
  chmod 600 "$home/.ssh/authorized_keys";
fi"#
    )
}

pub(crate) fn build_sidecar_ssh_key_install_command(username: &str, public_key: &str) -> String {
    let user_arg = shell_escape(username);
    let key_arg = shell_escape(public_key);
    format!(
        "set -eu; user={user_arg}; \
home=$(getent passwd \"${{user}}\" | cut -d: -f6); \
if [ -z \"$home\" ]; then echo \"User ${{user}} does not exist\" >&2; exit 1; fi; \
mkdir -p \"$home/.ssh\"; chmod 700 \"$home/.ssh\"; \
if ! grep -qxF {key_arg} \"$home/.ssh/authorized_keys\" 2>/dev/null; then \
    echo {key_arg} >> \"$home/.ssh/authorized_keys\"; \
fi; chmod 600 \"$home/.ssh/authorized_keys\""
    )
}

pub(crate) fn build_sidecar_ssh_key_revoke_command(username: &str, public_key: &str) -> String {
    let user_arg = shell_escape(username);
    let key_arg = shell_escape(public_key);
    format!(
        "set -eu; user={user_arg}; \
home=$(getent passwd \"${{user}}\" | cut -d: -f6); \
if [ -z \"$home\" ]; then echo \"User ${{user}} does not exist\" >&2; exit 1; fi; \
if [ -f \"$home/.ssh/authorized_keys\" ]; then \
    tmp=$(mktemp /tmp/authorized_keys.XXXXXX); \
    grep -vxF {key_arg} \"$home/.ssh/authorized_keys\" > \"$tmp\" || true; \
    mv \"$tmp\" \"$home/.ssh/authorized_keys\"; chmod 600 \"$home/.ssh/authorized_keys\"; \
fi"
    )
}
