//! Local T3 Code (`EvilBorsch/t3code`) inspection, install/update, and harness
//! discovery for Denial Settings.

use std::collections::BTreeMap;
use std::fs;
use std::net::{SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use serde_json::Value;

pub const DEFAULT_REPO_URL: &str = "https://github.com/EvilBorsch/t3code.git";
pub const DEFAULT_BRANCH: &str = "main";
const DEFAULT_PORT: u16 = 3773;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelInfo {
    pub slug: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderInfo {
    pub instance_id: String,
    pub driver: String,
    pub display_name: String,
    pub enabled: bool,
    pub ready: bool,
    pub models: Vec<ModelInfo>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    pub install_dir: PathBuf,
    pub installed: bool,
    pub commit: Option<String>,
    pub branch: Option<String>,
    pub server_running: bool,
    pub gigaam_installed: bool,
    pub providers: Vec<ProviderInfo>,
}

pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/home/dmitry"))
}

pub fn resolve_install_dir(override_dir: Option<&str>) -> PathBuf {
    let raw = override_dir.unwrap_or("").trim();
    if raw.is_empty() {
        return home_dir().join(".local/share/t3code-evilborsch");
    }
    if raw == "~" {
        return home_dir();
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return home_dir().join(rest);
    }
    PathBuf::from(raw)
}

pub fn probe(override_dir: Option<&str>) -> Status {
    let install_dir = resolve_install_dir(override_dir);
    let server_bin = install_dir.join("app/apps/server/dist/bin.mjs");
    let installed = server_bin.is_file();
    let app_dir = install_dir.join("app");
    let state_dir = if install_dir.join("state").is_dir() {
        install_dir.join("state")
    } else {
        home_dir().join(".t3")
    };

    let (commit, branch) = if app_dir.join(".git").exists() {
        (git_short_commit(&app_dir), git_branch(&app_dir))
    } else {
        (None, None)
    };

    let server_running = tcp_open(DEFAULT_PORT);
    let gigaam_installed = state_dir
        .join("userdata/speech/models/gigaam-v3-e2e-rnnt-Q8_0.gguf")
        .is_file();
    let providers = read_providers(&state_dir);

    Status {
        install_dir,
        installed,
        commit,
        branch,
        server_running,
        gigaam_installed,
        providers,
    }
}

fn tcp_open(port: u16) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(220)).is_ok()
}

fn git_short_commit(app_dir: &Path) -> Option<String> {
    let out = Command::new("git")
        .args(["-C", &app_dir.to_string_lossy(), "rev-parse", "--short", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

fn git_branch(app_dir: &Path) -> Option<String> {
    let out = Command::new("git")
        .args(["-C", &app_dir.to_string_lossy(), "rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

fn read_providers(state_dir: &Path) -> Vec<ProviderInfo> {
    let mut by_id: BTreeMap<String, ProviderInfo> = BTreeMap::new();

    // 1. Read configured providerInstances from userdata/settings.json.
    let settings_path = state_dir.join("userdata/settings.json");
    if let Ok(text) = fs::read_to_string(&settings_path)
        && let Ok(Value::Object(root)) = serde_json::from_str::<Value>(&text)
        && let Some(Value::Object(instances)) = root.get("providerInstances")
    {
        for (instance_id, raw) in instances {
            let Some(obj) = raw.as_object() else { continue };
            let driver = obj
                .get("driver")
                .and_then(Value::as_str)
                .unwrap_or(instance_id)
                .to_owned();
            let display_name = obj
                .get("displayName")
                .and_then(Value::as_str)
                .unwrap_or(instance_id)
                .to_owned();
            let enabled = obj.get("enabled").and_then(Value::as_bool).unwrap_or(true);
            by_id.insert(
                instance_id.clone(),
                ProviderInfo {
                    instance_id: instance_id.clone(),
                    driver,
                    display_name,
                    enabled,
                    ready: enabled,
                    models: Vec::new(),
                },
            );
        }
    }

    // 2. Enrich with cached provider snapshots in state/caches/*.json.
    let caches_dir = state_dir.join("caches");
    if let Ok(entries) = fs::read_dir(&caches_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(val) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if let Some(parsed) = parse_provider_cache(stem, &val) {
                let entry = by_id
                    .entry(parsed.instance_id.clone())
                    .or_insert_with(|| parsed.clone());
                if entry.models.is_empty() {
                    entry.models = parsed.models;
                }
                entry.ready = parsed.ready;
                if entry.display_name == entry.instance_id && !parsed.display_name.is_empty() {
                    entry.display_name = parsed.display_name;
                }
            }
        }
    }

    let mut list: Vec<ProviderInfo> = by_id.into_values().collect();
    // Show enabled/ready providers with models first.
    list.sort_by(|a, b| {
        b.ready
            .cmp(&a.ready)
            .then(b.enabled.cmp(&a.enabled))
            .then(b.models.len().cmp(&a.models.len()))
            .then(a.display_name.cmp(&b.display_name))
    });
    list
}

fn parse_provider_cache(stem: &str, value: &Value) -> Option<ProviderInfo> {
    let obj = value.as_object()?;
    let models_arr = obj.get("models").and_then(Value::as_array)?;
    let mut models = Vec::new();
    for m in models_arr {
        let Some(mo) = m.as_object() else { continue };
        let Some(slug) = mo.get("slug").and_then(Value::as_str) else {
            continue;
        };
        let name = mo
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .unwrap_or(slug);
        let is_default = mo.get("isDefault").and_then(Value::as_bool).unwrap_or(false);
        models.push(ModelInfo {
            slug: slug.to_owned(),
            name: name.to_owned(),
            is_default,
        });
    }
    let display_name = obj
        .get("displayName")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(stem)
        .to_owned();
    let enabled = obj.get("enabled").and_then(Value::as_bool).unwrap_or(false);
    let status = obj.get("status").and_then(Value::as_str).unwrap_or("");
    let ready = enabled && (status == "ready" || status == "warning");
    Some(ProviderInfo {
        instance_id: stem.to_owned(),
        driver: stem.to_owned(),
        display_name,
        enabled,
        ready,
        models,
    })
}

/// Launches the installed T3 Code desktop application detached.
pub fn launch(override_dir: Option<&str>) -> Result<(), String> {
    let install_dir = resolve_install_dir(override_dir);
    let launcher = install_dir.join("launch");
    if !launcher.is_file() {
        return Err("T3 Code launcher is missing".to_owned());
    }
    Command::new(&launcher)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("Failed to launch T3 Code: {err}"))
}

/// Clones or fast-forwards `EvilBorsch/t3code` (`main`), builds web + server +
/// desktop with Vite+ (`vp`), and refreshes the `launch` wrapper and desktop
/// entry. Runs synchronously — call on a background executor.
pub fn install_or_update(
    override_dir: Option<&str>,
    repo_url: &str,
    branch: &str,
) -> Result<String, String> {
    let install_dir = resolve_install_dir(override_dir);
    let repo = if repo_url.trim().is_empty() {
        DEFAULT_REPO_URL
    } else {
        repo_url.trim()
    };
    let branch = if branch.trim().is_empty() {
        DEFAULT_BRANCH
    } else {
        branch.trim()
    };

    let script = r#"
set -eu
install_root="$1"
repo_url="$2"
branch="$3"
export PATH="$HOME/.vite-plus/bin:$HOME/.local/bin:$PATH"

if ! command -v vp >/dev/null 2>&1; then
  curl -fsSL https://vite.plus | bash
  export PATH="$HOME/.vite-plus/bin:$HOME/.local/bin:$PATH"
fi

mkdir -p "$install_root/state" "$install_root/config" "$install_root/data" "$install_root/cache"
if [ ! -d "$install_root/app/.git" ]; then
  git clone --branch "$branch" --single-branch "$repo_url" "$install_root/app"
else
  git -C "$install_root/app" fetch origin "$branch"
  git -C "$install_root/app" checkout "$branch"
  git -C "$install_root/app" pull --ff-only origin "$branch"
fi

cd "$install_root/app"
vp install
vp run --filter @t3tools/web --filter @t3tools/server --filter @t3tools/desktop build
commit_hash=$(git -C "$install_root/app" rev-parse HEAD)
short_hash=$(git -C "$install_root/app" rev-parse --short HEAD)

cat > "$install_root/launch" <<LAUNCH_EOF
#!/bin/sh
set -eu
install_root=\$(CDPATH='' cd -- "\$(dirname -- "\$0")" && pwd)
export T3CODE_HOME="\$install_root/state"
export XDG_CONFIG_HOME="\$install_root/config"
export XDG_DATA_HOME="\$install_root/data"
export XDG_CACHE_HOME="\$install_root/cache"
export T3CODE_DESKTOP_LAUNCHER_PATH="\$install_root/launch"
export T3CODE_COMMIT_HASH=$commit_hash
if ! command -v codex >/dev/null 2>&1 && [ -x /usr/lib/chatgpt/resources/codex ]; then
    export PATH="\$PATH:/usr/lib/chatgpt/resources"
fi
cd "\$install_root/app/apps/desktop"
exec ./node_modules/electron/dist/electron --disable-setuid-sandbox . "\$@"
LAUNCH_EOF
chmod +x "$install_root/launch"

mkdir -p "$HOME/.local/share/applications"
cat > "$HOME/.local/share/applications/com.t3tools.T3Code.desktop" <<DESKTOP_EOF
[Desktop Entry]
Type=Application
Name=T3 Code (EvilBorsch)
Comment=Control coding agents with T3 Code
Exec="$install_root/launch" %U
Icon=$install_root/app/assets/prod/t3-black-web-icon-512x512.png
StartupWMClass=com.t3tools.T3Code
Terminal=false
Categories=Development;
DESKTOP_EOF

printf '%s\n' "$short_hash"
"#;

    let out = Command::new("sh")
        .arg("-c")
        .arg(script)
        .arg("t3code-install")
        .arg(install_dir.to_string_lossy().as_ref())
        .arg(repo)
        .arg(branch)
        .output()
        .map_err(|err| format!("Could not start installer: {err}"))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        let tail: String = stderr
            .lines()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join(" | ");
        return Err(if tail.is_empty() {
            format!("Installer exited with {}", out.status)
        } else {
            tail
        });
    }
    let commit = String::from_utf8_lossy(&out.stdout)
        .lines()
        .last()
        .unwrap_or("")
        .trim()
        .to_owned();
    Ok(commit)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_provider_cache_snapshot() {
        let raw = json!({
            "displayName": "Antigravity",
            "enabled": true,
            "status": "ready",
            "models": [
                {"slug": "gemini-4-neon", "name": "Gemini 4 Neon", "isDefault": true},
                {"slug": "gemini-3-flash", "name": "Gemini 3 Flash", "isDefault": false}
            ]
        });
        let info = parse_provider_cache("antigravity", &raw).expect("cache parses");
        assert_eq!(info.instance_id, "antigravity");
        assert_eq!(info.display_name, "Antigravity");
        assert!(info.ready);
        assert_eq!(info.models.len(), 2);
        assert_eq!(info.models[0].slug, "gemini-4-neon");
        assert!(info.models[0].is_default);
    }
}
