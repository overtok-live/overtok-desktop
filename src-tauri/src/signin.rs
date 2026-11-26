//! Signing in through the system browser (Google refuses to sign in inside an app's window):
//! the app opens the dashboard's `/desktop/connect` with a random `state`, the browser comes back
//! with `<scheme>://signed-in?ticket=…&state=…`, and the window signs in with the one-time ticket.

use url::Url;

/// The ticket a sign-in link carries, when it's this build's link and the state the app asked
/// for; None for anything else (another app's link, an old or forged one).
pub fn parse_deep_link(link: &str, scheme: &str, expected_state: Option<&str>) -> Option<String> {
    let url = Url::parse(link).ok()?;
    if url.scheme() != scheme || url.host_str() != Some("signed-in") {
        return None;
    }
    let expected = expected_state?;
    let mut ticket = None;
    let mut state = None;
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "ticket" => ticket = Some(value.into_owned()),
            "state" => state = Some(value.into_owned()),
            _ => {}
        }
    }
    if state.as_deref() != Some(expected) {
        return None;
    }
    ticket.filter(|ticket| !ticket.is_empty() && ticket.len() < 4096)
}

/// Where the browser goes to sign in for the app.
pub fn connect_url(app_url: &str, state: &str, scheme: &str) -> String {
    let mut url = Url::parse(&format!("{app_url}/desktop/connect")).expect("app url");
    url.query_pairs_mut()
        .append_pair("state", state)
        .append_pair("scheme", scheme);
    url.to_string()
}

/// Where the window goes to sign in with a ticket, then on to the dashboard.
pub fn ticket_url(app_url: &str, ticket: &str) -> String {
    let mut url = Url::parse(&format!("{app_url}/sign-in")).expect("app url");
    url.query_pairs_mut()
        .append_pair("__clerk_ticket", ticket)
        .append_pair("redirect_url", &format!("{app_url}/live"));
    url.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_ticket_of_its_own_link() {
        assert_eq!(
            parse_deep_link("overtok://signed-in?ticket=abc&state=s1", "overtok", Some("s1")),
            Some("abc".into())
        );
    }

    #[test]
    fn refuses_a_wrong_state_scheme_or_place() {
        let link = "overtok://signed-in?ticket=abc&state=s1";
        assert_eq!(parse_deep_link(link, "overtok", Some("other")), None);
        assert_eq!(parse_deep_link(link, "overtok", None), None);
        assert_eq!(parse_deep_link(link, "overtok-dev", Some("s1")), None);
        assert_eq!(parse_deep_link("overtok://elsewhere?ticket=abc&state=s1", "overtok", Some("s1")), None);
        assert_eq!(parse_deep_link("overtok://signed-in?state=s1", "overtok", Some("s1")), None);
    }

    #[test]
    fn builds_the_connect_and_ticket_addresses() {
        assert_eq!(
            connect_url("https://app.overtok.live", "s 1", "overtok"),
            "https://app.overtok.live/desktop/connect?state=s+1&scheme=overtok"
        );
        assert!(ticket_url("https://app.overtok.live", "t").starts_with(
            "https://app.overtok.live/sign-in?__clerk_ticket=t&redirect_url=https%3A%2F%2Fapp.overtok.live%2Flive"
        ));
    }
}
