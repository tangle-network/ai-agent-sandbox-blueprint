//! Git automation around a workflow's harness run: clone the repository and
//! check out the work branch before the task, then commit, push and optionally
//! open a GitHub pull request after it. Every git and GitHub call runs inside
//! the sandbox, so the push token stays in the sandbox environment variable
//! named by `token_env` and never passes through the operator.

use serde::Deserialize;
use serde_json::json;

use crate::SandboxExecRequest;
use crate::jobs::exec::run_exec_request;

#[derive(Clone, Debug, Deserialize)]
pub struct WorkflowGitSpec {
    pub repo_url: String,
    /// Branch the task's changes are committed and pushed to. An existing
    /// remote branch is continued, so recurring runs extend one branch.
    pub branch: String,
    /// Branch to start from and target with the pull request. Defaults to the
    /// repository's default branch.
    #[serde(default)]
    pub base_branch: Option<String>,
    /// Name of the sandbox environment variable that holds the token used for
    /// clone, push and the pull request API.
    #[serde(default)]
    pub token_env: Option<String>,
    #[serde(default)]
    pub open_pr: bool,
}

pub struct GitCheckout {
    pub dir: String,
    pub base_branch: String,
    start_commit: String,
}

pub struct GitPublish {
    /// Pushed commit, or `None` when the task left the checkout unchanged.
    pub commit: Option<String>,
    pub pull_request_url: Option<String>,
}

fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

impl WorkflowGitSpec {
    fn credential_helper(&self) -> Result<Option<String>, String> {
        let Some(name) = self.token_env.as_deref().filter(|n| !n.is_empty()) else {
            return Ok(None);
        };
        let valid = name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
        if !valid {
            return Err(format!(
                "git.token_env '{name}' is not an environment variable name"
            ));
        }
        Ok(Some(format!(
            "!f() {{ echo username=x-access-token; echo \"password=${name}\"; }}; f"
        )))
    }

    /// `owner/repo` when the repository is hosted on github.com.
    fn github_repo(&self) -> Option<String> {
        let path = self.repo_url.strip_prefix("https://github.com/")?;
        let path = path.trim_end_matches('/').trim_end_matches(".git");
        let safe = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/');
        (path.split('/').count() == 2 && path.chars().all(safe)).then(|| path.to_string())
    }

    fn checkout_dir(&self, workflow_id: u64) -> String {
        let name = self
            .repo_url
            .trim_end_matches('/')
            .trim_end_matches(".git")
            .rsplit('/')
            .next()
            .unwrap_or_default();
        let name: String = name
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
            .collect();
        let name = name.trim_start_matches('.');
        format!(
            "wf-{workflow_id}-{}",
            if name.is_empty() { "repo" } else { name }
        )
    }
}

async fn sandbox_shell(
    sidecar_url: &str,
    token: &str,
    cwd: &str,
    command: String,
) -> Result<String, String> {
    let request = SandboxExecRequest {
        sidecar_url: sidecar_url.to_string(),
        command,
        cwd: cwd.to_string(),
        env_json: String::new(),
        timeout_ms: 600_000,
    };
    let response = run_exec_request(&request, token).await?;
    if response.exit_code != 0 {
        return Err(format!(
            "git step failed (exit {}): {}",
            response.exit_code,
            response.stderr.trim()
        ));
    }
    Ok(response.stdout)
}

/// Clone the repository into the sandbox workspace and check out the work branch.
pub async fn prepare_git_checkout(
    sidecar_url: &str,
    token: &str,
    spec: &WorkflowGitSpec,
    workflow_id: u64,
) -> Result<GitCheckout, String> {
    if spec.repo_url.trim().is_empty() || spec.repo_url.starts_with('-') {
        return Err("git.repo_url is required".to_string());
    }
    let helper = spec.credential_helper()?;
    if helper.is_some() && !spec.repo_url.starts_with("https://") {
        return Err("git.token_env needs an https:// repo_url".to_string());
    }
    let auth = helper
        .as_deref()
        .map(|h| format!("-c credential.helper={}", sh_quote(h)))
        .unwrap_or_default();
    let dir = sh_quote(&spec.checkout_dir(workflow_id));
    let branch = sh_quote(&spec.branch);
    let mut script = format!(
        "set -eu\ngit check-ref-format --branch {branch} >/dev/null\nrm -rf {dir}\n\
         git {auth} clone --quiet {url} {dir}\ncd {dir}\n",
        url = sh_quote(&spec.repo_url),
    );
    if let Some(h) = &helper {
        script.push_str(&format!("git config credential.helper {}\n", sh_quote(h)));
    }
    if let Some(base) = spec.base_branch.as_deref().filter(|b| !b.is_empty()) {
        let base = sh_quote(base);
        script.push_str(&format!(
            "git check-ref-format --branch {base} >/dev/null\ngit checkout --quiet {base} --\n"
        ));
    }
    script.push_str(&format!(
        "base=$(git rev-parse --abbrev-ref HEAD)\n\
         if git rev-parse --verify --quiet origin/{branch} >/dev/null; then \
         git checkout --quiet -B {branch} origin/{branch}; else git checkout --quiet -B {branch}; fi\n\
         printf '%s\\n%s\\n%s\\n' \"$(pwd)\" \"$base\" \"$(git rev-parse HEAD)\"\n"
    ));
    let out = sandbox_shell(sidecar_url, token, "", script).await?;
    // A login shell's profile may print first, so read the last three lines.
    let mut lines = out.lines().rev().map(str::trim);
    match (lines.next(), lines.next(), lines.next()) {
        (Some(start), Some(base), Some(dir)) if !start.is_empty() => Ok(GitCheckout {
            dir: dir.to_string(),
            base_branch: base.to_string(),
            start_commit: start.to_string(),
        }),
        _ => Err(format!("git checkout returned unexpected output: {out}")),
    }
}

/// Commit the task's changes, push the work branch, and open (or reuse) a
/// pull request when `open_pr` is set for a GitHub repository.
pub async fn publish_git_changes(
    sidecar_url: &str,
    token: &str,
    spec: &WorkflowGitSpec,
    checkout: &GitCheckout,
    message: &str,
) -> Result<GitPublish, String> {
    let branch = sh_quote(&spec.branch);
    let script = format!(
        "set -eu\ngit add -A\ngit diff --cached --quiet || git -c user.name=sandbox-agent \
         -c user.email=sandbox-agent@users.noreply.github.com commit --quiet -m {msg}\n\
         head=$(git rev-parse HEAD)\npushed=\nif [ \"$head\" != {start} ]; then \
         git push --quiet origin HEAD:refs/heads/{branch}; pushed=$head; fi\n\
         printf '%s\\n%s\\n' \"$pushed\" \"$(git rev-list --count origin/{base}..HEAD)\"\n",
        msg = sh_quote(message),
        start = sh_quote(&checkout.start_commit),
        base = sh_quote(&checkout.base_branch),
    );
    let out = sandbox_shell(sidecar_url, token, &checkout.dir, script).await?;
    let mut lines = out.lines().rev().map(str::trim);
    let ahead = lines.next().unwrap_or_default();
    let commit = lines.next().filter(|c| !c.is_empty()).map(str::to_string);

    // Retried on every run while the branch is ahead of base, so a failed
    // pull request call recovers without a new commit.
    let mut pull_request_url = None;
    if spec.open_pr && ahead != "0" {
        let (Some(repo), Some(token_env)) = (spec.github_repo(), spec.token_env.as_deref()) else {
            return Err(
                "git.open_pr needs a https://github.com/ repo_url and token_env".to_string(),
            );
        };
        spec.credential_helper()?;
        let owner = repo.split('/').next().unwrap_or_default();
        let body = json!({
            "title": message,
            "head": spec.branch,
            "base": checkout.base_branch,
            "body": "Opened by a sandbox blueprint workflow run.",
        });
        let script = format!(
            "set -eu\napi=https://api.github.com/repos/{repo}/pulls\n\
             auth=\"Authorization: Bearer ${token_env}\"\n\
             head=$(python3 -c 'import sys,urllib.parse; print(urllib.parse.quote(sys.argv[1], safe=\"\"))' {head})\n\
             url=$(curl -fsS -H \"$auth\" \"$api?state=open&head=$head\" | python3 -c \
             'import json,sys; p=json.load(sys.stdin); print(p[0][\"html_url\"] if p else \"\")')\n\
             [ -n \"$url\" ] || url=$(curl -fsS -H \"$auth\" -d {body} \"$api\" | \
             python3 -c 'import json,sys; print(json.load(sys.stdin)[\"html_url\"])')\necho \"$url\"\n",
            head = sh_quote(&format!("{owner}:{}", spec.branch)),
            body = sh_quote(&body.to_string()),
        );
        let out = sandbox_shell(sidecar_url, token, &checkout.dir, script).await?;
        pull_request_url = Some(out.trim().to_string()).filter(|u| !u.is_empty());
    }
    Ok(GitPublish {
        commit,
        pull_request_url,
    })
}
