//! Which Overtok this build is: the app (`live.overtok.desktop`) or Overtok Dev, and so which
//! dashboard it shows and which link scheme brings it back from the browser.

use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Channel {
    pub name: &'static str,
    /// The dashboard the window shows, without a trailing slash.
    pub app_url: String,
    /// What `overtok://…` links are called in this build.
    pub scheme: &'static str,
}

impl Channel {
    /// The channel for this bundle identifier. A debug build of Overtok Dev may point at another
    /// dashboard (a local one) with `OVERTOK_APP_URL`.
    pub fn for_identifier(identifier: &str, app_url_override: Option<String>) -> Channel {
        if identifier.ends_with(".dev") {
            let app_url = app_url_override
                .filter(|_| cfg!(debug_assertions))
                .unwrap_or_else(|| "https://app.dev.overtok.live".into());
            Channel {
                name: "dev",
                app_url: app_url.trim_end_matches('/').to_string(),
                scheme: "overtok-dev",
            }
        } else {
            Channel {
                name: "prod",
                app_url: "https://app.overtok.live".into(),
                scheme: "overtok",
            }
        }
    }

    /// The dashboard's own origin, which the window may show.
    pub fn app_origin(&self) -> String {
        Url::parse(&self.app_url)
            .map(|url| url.origin().ascii_serialization())
            .unwrap_or_default()
    }

    /// Whether the window may go to `url` itself: the dashboard, its sign-in provider (Clerk) and
    /// the app's own pages. Anything else opens in the system browser.
    pub fn stays_in_window(&self, url: &Url) -> bool {
        if matches!(url.scheme(), "tauri" | "asset" | "about") {
            return true;
        }
        let host = url.host_str().unwrap_or_default();
        if host == "tauri.localhost" {
            return true;
        }
        if url.origin().ascii_serialization() == self.app_origin() {
            return true;
        }
        url.scheme() == "https"
            && (host.ends_with(".overtok.live") || host.ends_with(".clerk.accounts.dev"))
            && host != "overtok.live"
            && !host.starts_with("downloads.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prod_shows_the_live_dashboard_and_ignores_overrides() {
        let channel = Channel::for_identifier("live.overtok.desktop", Some("http://x".into()));
        assert_eq!(channel.app_url, "https://app.overtok.live");
        assert_eq!(channel.scheme, "overtok");
    }

    #[test]
    fn dev_has_its_own_scheme() {
        let channel = Channel::for_identifier("live.overtok.desktop.dev", None);
        assert_eq!(channel.app_url, "https://app.dev.overtok.live");
        assert_eq!(channel.scheme, "overtok-dev");
    }

    #[test]
    fn keeps_the_dashboard_and_sends_the_rest_to_the_browser() {
        let channel = Channel::for_identifier("live.overtok.desktop", None);
        let stays = |url: &str| channel.stays_in_window(&Url::parse(url).unwrap());
        assert!(stays("https://app.overtok.live/live"));
        assert!(stays("https://clerk.overtok.live/v1/client"));
        assert!(stays("tauri://localhost/index.html"));
        assert!(!stays("https://overtok.live/help"));
        assert!(!stays("https://checkout.dodopayments.com/x"));
        assert!(!stays("https://accounts.google.com/o/oauth2"));
        assert!(!stays("http://app.overtok.live/live"));
    }
}
