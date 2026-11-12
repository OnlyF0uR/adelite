//! What GitHub says about the newest release, and the file in it for this
//! system. Releases are published by `.github/workflows/release.yml`.

use serde_json::Value;

use crate::Error;
use crate::fetch;
use crate::version::Version;

/// Where the releases are published: `owner/name` on GitHub.
pub const REPOSITORY: &str = "OnlyF0uR/adelite";

/// The end of this system's download's name, after `Adelite-<version>-`;
/// `None` where no download is built.
pub const SUFFIX: Option<&str> = if cfg!(all(windows, target_arch = "x86_64")) {
    Some("windows-x64-setup.exe")
} else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    Some("macos-arm64.tar.gz")
} else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    Some("linux-x64.tar.gz")
} else {
    None
};

/// The name of this system's download of `version`.
#[must_use]
pub fn asset_name(version: Version) -> Option<String> {
    SUFFIX.map(|suffix| format!("Adelite-{version}-{suffix}"))
}

/// The releases' page, for when a release does not name its own.
#[must_use]
pub fn releases_page() -> String {
    format!("https://github.com/{REPOSITORY}/releases")
}

/// A file of a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    /// Its name, which the signature covers.
    pub name: String,
    /// Where it is downloaded from (HTTPS).
    pub url: String,
    /// Its size in bytes, as GitHub says.
    pub size: u64,
}

/// A published release, as far as updating needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// Its version.
    pub version: Version,
    /// Its page on GitHub, which says what changed.
    pub page: String,
    /// This system's download and its signature; `None` when the release has
    /// either missing for this system.
    pub download: Option<(Asset, Asset)>,
}

/// The newest published release; `None` when there is none yet. GitHub
/// leaves drafts and pre-releases out.
///
/// # Errors
/// GitHub could not be asked, or its answer was not a release.
pub fn latest() -> Result<Option<Release>, Error> {
    let url = format!("https://api.github.com/repos/{REPOSITORY}/releases/latest");
    let (status, body) = fetch::get(&url, "application/vnd.github+json")?;
    match status {
        200 => {}
        404 => return Ok(None),
        403 | 429 => {
            return Err(Error::new("GitHub is turning away requests from this network for now. Try again later."));
        }
        _ => return Err(Error::new(format!("GitHub answered {status}. Try again later."))),
    }
    let json: Value = serde_json::from_slice(&body)
        .map_err(|e| Error::logged("GitHub's answer could not be read.", e.to_string()))?;
    parse(&json).map(Some).ok_or_else(|| Error::new("The newest release has no version Adelite understands."))
}

/// Reads GitHub's description of a release; `None` when its tag is not a version.
#[must_use]
pub fn parse(json: &Value) -> Option<Release> {
    let version = Version::parse(json["tag_name"].as_str()?)?;
    let page =
        json["html_url"].as_str().filter(|url| url.starts_with("https://")).map_or_else(releases_page, str::to_owned);
    let assets = json["assets"].as_array().map(Vec::as_slice).unwrap_or_default();
    let find = |name: &str| {
        let asset = assets.iter().find(|asset| asset["name"].as_str() == Some(name))?;
        let url = asset["browser_download_url"].as_str().filter(|url| url.starts_with("https://"))?;
        Some(Asset { name: name.to_owned(), url: url.to_owned(), size: asset["size"].as_u64().unwrap_or(0) })
    };
    let download = asset_name(version).and_then(|name| Some((find(&name)?, find(&format!("{name}.sig"))?)));
    Some(Release { version, page, download })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn asset(name: &str) -> Value {
        json!({"name": name, "size": 1234, "browser_download_url": format!("https://github.com/x/{name}")})
    }

    #[test]
    fn a_release_offers_this_systems_download_and_its_signature() {
        let names = ["windows-x64-setup.exe", "macos-arm64.tar.gz", "macos-arm64.dmg", "linux-x64.tar.gz"];
        let mut assets: Vec<Value> = names.iter().map(|n| asset(&format!("Adelite-0.2.0-{n}"))).collect();
        assets.extend(names.iter().map(|n| asset(&format!("Adelite-0.2.0-{n}.sig"))));
        let json =
            json!({"tag_name": "v0.2.0", "html_url": "https://github.com/x/releases/tag/v0.2.0", "assets": assets});
        let release = parse(&json).expect("a release");
        assert_eq!(release.version, Version(0, 2, 0));
        assert_eq!(release.page, "https://github.com/x/releases/tag/v0.2.0");
        match (asset_name(release.version), release.download) {
            (Some(name), Some((file, signature))) => {
                assert_eq!(file.name, name);
                assert_eq!(signature.name, format!("{name}.sig"));
                assert_eq!(file.size, 1234);
                assert!(file.url.ends_with(&name));
            }
            (None, None) => {}
            other => panic!("download and system disagree: {other:?}"),
        }
    }

    #[test]
    fn a_release_without_this_systems_file_or_signature_offers_nothing() {
        let Some(name) = asset_name(Version(0, 2, 0)) else { return };
        // Unsigned: not offered.
        let json = json!({"tag_name": "v0.2.0", "assets": [asset(&name)]});
        let release = parse(&json).expect("a release");
        assert_eq!(release.download, None);
        assert_eq!(release.page, releases_page());
        // Not over HTTPS: not offered.
        let mut plain = asset(&name);
        plain["browser_download_url"] = json!("http://example.com/file");
        let json = json!({"tag_name": "v0.2.0", "assets": [plain, asset(&format!("{name}.sig"))]});
        assert_eq!(parse(&json).expect("a release").download, None);
    }

    /// Asks GitHub for real: `cargo test -p adelite-update github -- --ignored`
    #[test]
    #[ignore = "asks GitHub over the network; run by hand"]
    fn github_answers_about_the_newest_release() {
        match latest() {
            Ok(release) => println!("newest release: {release:?}"),
            Err(e) => panic!("{e} {}", e.detail()),
        }
    }

    #[test]
    fn something_that_is_not_a_release_is_not_read_as_one() {
        assert_eq!(parse(&json!({})), None);
        assert_eq!(parse(&json!({"tag_name": "nightly"})), None);
        assert_eq!(parse(&json!({"tag_name": 3})), None);
        let bare = parse(&json!({"tag_name": "v1.0.0", "assets": "nonsense"})).expect("a release");
        assert_eq!(bare.download, None);
    }
}
