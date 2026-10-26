//! Update check (off unless the user switches it on, or presses "Check now").
//!
//! Asks GitHub for the latest published release of this app and compares its version with this
//! one. Nothing is downloaded or installed: the app only says that a new version exists and
//! links to it. Drafts and pre-releases are not "latest", so they are never offered.
//! GitHub sees the request (IP address, "open-adi2-control/<version>" as user agent), nothing else
//! is sent.

use serde::{Deserialize, Serialize};
use std::time::Duration;

pub const REPO_URL: &str = "https://github.com/shells-dw/open-adi2-control";
const API_URL: &str = "https://api.github.com/repos/shells-dw/open-adi2-control/releases/latest";

/// How often a running app checks again (it also checks shortly after starting).
pub const EVERY: Duration = Duration::from_secs(24 * 3600);

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Release {
    /// "0.1.4"
    pub version: String,
    pub name: String,
    /// release notes (Markdown as written on GitHub)
    pub notes: String,
    /// the release page
    pub url: String,
    /// ISO date
    pub published: String,
    /// the package for this installation, when the release has one
    pub download: Option<Asset>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
    html_url: String,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

/// "v0.1.10" / "0.1.10" → (0, 1, 10); anything after "-" or "+" is ignored.
pub fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim().trim_start_matches(['v', 'V']);
    let core = s.split(['-', '+']).next()?;
    let mut it = core.split('.').map(|p| p.parse::<u64>().ok());
    let v = (it.next()??, it.next().flatten().unwrap_or(0), it.next().flatten().unwrap_or(0));
    Some(v)
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// How this copy was installed: "appimage", "deb" or "other" (built from source …).
/// How this copy was installed: "appimage", "flatpak", "deb", "rpm", "pacman" (AUR) or "other".
pub fn install_kind() -> &'static str {
    use std::path::Path;
    if std::env::var_os("APPIMAGE").is_some_and(|v| !v.is_empty()) {
        return "appimage";
    }
    if std::env::var_os("FLATPAK_ID").is_some() {
        return "flatpak";
    }
    let exe = std::env::current_exe().ok().and_then(|p| std::fs::canonicalize(p).ok());
    if exe.as_deref() != Some(Path::new("/usr/bin/open-adi2-control")) {
        return "other";
    }
    if Path::new("/var/lib/dpkg/info/open-adi2-control.list").exists() {
        "deb"
    } else if Path::new("/var/lib/pacman/local").is_dir() {
        "pacman"
    } else if Path::new("/var/lib/rpm").is_dir() || Path::new("/usr/lib/sysimage/rpm").is_dir() {
        "rpm"
    } else {
        "other"
    }
}

fn arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        a => a,
    }
}

/// Only links into this project's releases are ever opened.
pub fn trusted(url: &str) -> bool {
    url.starts_with(&format!("{REPO_URL}/releases/")) && !url.contains("..")
}

/// The release file for this kind of installation: `…_amd64.deb`, `…_amd64.AppImage`,
/// `….x86_64.rpm` (rpm names use the kernel's architecture names).
fn pick_asset(assets: &[GhAsset], kind: &str) -> Option<Asset> {
    let (ext, arch) = match kind {
        "deb" => (".deb", format!("_{}", arch())),
        "appimage" => (".AppImage", format!("_{}", arch())),
        "rpm" => (".rpm", format!(".{}", std::env::consts::ARCH)),
        _ => return None,
    };
    assets
        .iter()
        .find(|a| a.name.ends_with(ext) && a.name.contains(&arch) && trusted(&a.browser_download_url))
        .map(|a| Asset { name: a.name.clone(), url: a.browser_download_url.clone() })
}

fn api_url() -> String {
    // debug builds only: a local stand-in for the tests
    if cfg!(debug_assertions) {
        if let Ok(u) = std::env::var("OPEN_ADI2_CONTROL_UPDATE_URL") {
            return u;
        }
    }
    API_URL.to_string()
}

/// The latest published release. Err is a short reason for the settings page.
pub fn latest() -> Result<Release, String> {
    let url = api_url();
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        // HTTPS_PROXY / ALL_PROXY as set on the system (not for the local test stand-in)
        .try_proxy_from_env(url.starts_with("https://"))
        .user_agent(&format!("open-adi2-control/{}", env!("CARGO_PKG_VERSION")))
        .build();
    let resp = agent
        .get(&url)
        .set("Accept", "application/vnd.github+json")
        .set("X-GitHub-Api-Version", "2022-11-28")
        .call();
    let resp = match resp {
        Ok(r) => r,
        Err(ureq::Error::Status(404, _)) => return Err("no published release on GitHub yet".into()),
        Err(ureq::Error::Status(403 | 429, r)) if r.header("x-ratelimit-remaining") == Some("0") => {
            return Err("GitHub's request limit reached, trying again later".into())
        }
        Err(ureq::Error::Status(code, _)) => return Err(format!("GitHub answered {code}")),
        Err(ureq::Error::Transport(t)) => return Err(format!("can't reach GitHub ({})", t.kind())),
    };
    let r: GhRelease = resp.into_json().map_err(|e| format!("unexpected answer from GitHub ({e})"))?;
    if r.draft || r.prerelease {
        return Err("no published release on GitHub yet".into());
    }
    let version = r.tag_name.trim_start_matches(['v', 'V']).to_string();
    if parse_version(&version).is_none() {
        return Err(format!("unexpected version “{}” on GitHub", r.tag_name));
    }
    let url = if trusted(&r.html_url) { r.html_url } else { format!("{REPO_URL}/releases") };
    Ok(Release {
        name: r.name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("Open ADI-2 Control {version}")),
        notes: r.body.unwrap_or_default(),
        published: r.published_at.unwrap_or_default().chars().take(10).collect(),
        download: pick_asset(&r.assets, install_kind()),
        version,
        url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert_eq!(parse_version("v0.1.10"), Some((0, 1, 10)));
        assert_eq!(parse_version("1.2"), Some((1, 2, 0)));
        assert_eq!(parse_version("0.2.0-beta.1"), Some((0, 2, 0)));
        assert_eq!(parse_version("latest"), None);
        assert!(is_newer("v0.1.10", "0.1.9"));
        assert!(is_newer("0.2.0", "0.1.3"));
        assert!(!is_newer("v0.1.3", "0.1.3"));
        assert!(!is_newer("0.1.2", "0.1.3"));
        assert!(!is_newer("garbage", "0.1.3"));
    }

    #[test]
    fn assets_and_links() {
        let base = format!("{REPO_URL}/releases/download/v0.1.4/");
        let assets = vec![
            GhAsset { name: "open-adi2-control_0.1.4_arm64.deb".into(), browser_download_url: format!("{base}open-adi2-control_0.1.4_arm64.deb") },
            GhAsset { name: "open-adi2-control_0.1.4_amd64.deb".into(), browser_download_url: format!("{base}open-adi2-control_0.1.4_amd64.deb") },
            GhAsset { name: "open-adi2-control_0.1.4_amd64.AppImage".into(), browser_download_url: "https://example.com/x.AppImage".into() },
        ];
        let deb = pick_asset(&assets, "deb").unwrap();
        assert_eq!(deb.name, format!("open-adi2-control_0.1.4_{}.deb", arch()));
        assert!(pick_asset(&assets, "appimage").is_none(), "links outside the project are never offered");
        assert!(pick_asset(&assets, "other").is_none());
        let rpms = vec![
            GhAsset { name: "open-adi2-control-0.2.0-1.aarch64.rpm".into(), browser_download_url: format!("{base}open-adi2-control-0.2.0-1.aarch64.rpm") },
            GhAsset { name: "open-adi2-control-0.2.0-1.x86_64.rpm".into(), browser_download_url: format!("{base}open-adi2-control-0.2.0-1.x86_64.rpm") },
        ];
        assert_eq!(pick_asset(&rpms, "rpm").unwrap().name, format!("open-adi2-control-0.2.0-1.{}.rpm", std::env::consts::ARCH));
        assert!(pick_asset(&rpms, "pacman").is_none());
        assert!(trusted(&format!("{REPO_URL}/releases/tag/v0.1.4")));
        assert!(!trusted("https://github.com/someone-else/open-adi2-control/releases/tag/v1"));
        assert!(!trusted(&format!("{REPO_URL}/../evil")));
    }
}
