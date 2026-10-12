//! Code navigation keeps credentials only in live native pages.
use super::{FOLDER, TOKEN, WebUrl};

/// The address a VS Code page loads: where its tab last was, carrying the
/// token of the configured address `configured`, in place of any it had.
/// The server takes the token, sets its cookie, and redirects to the same
/// path and query without it, so a page keeps its folder across a restart
/// or a new token. An address on another server is left as it is.
#[cfg(any(target_os = "macos", windows, test))]
pub(super) fn with_token(page: &WebUrl, configured: &WebUrl) -> WebUrl {
    // The raw query text, so the rest of the page's address stays exactly
    // as the server wrote it.
    let is_token = |pair: &&str| is_parameter(pair, TOKEN);
    let Some(token) = configured
        .0
        .query()
        .and_then(|query| query.split('&').find(is_token))
    else {
        return page.clone();
    };
    if page.origin() != configured.origin() {
        return page.clone();
    }
    let query: Vec<&str> = page
        .0
        .query()
        .into_iter()
        .flat_map(|query| query.split('&'))
        .filter(|pair| !is_token(pair))
        .chain([token])
        .collect();
    let mut url = page.0.clone();
    url.set_query(Some(&query.join("&")));
    WebUrl::try_from(url.as_str()).unwrap_or_else(|_| page.clone())
}

/// Where `page`, on another server, is on `server`: the same path and
/// query, less any old token, which [`with_token`] adds back as the page
/// loads.
pub(super) fn on_server(page: &WebUrl, server: &WebUrl) -> WebUrl {
    let is_token = |pair: &&str| is_parameter(pair, TOKEN);
    let query: Vec<&str> = page
        .0
        .query()
        .into_iter()
        .flat_map(|query| query.split('&'))
        .filter(|pair| !is_token(pair))
        .collect();
    let mut url = server.0.clone();
    url.set_path(page.0.path());
    url.set_fragment(page.0.fragment());
    url.set_query((!query.is_empty()).then(|| query.join("&")).as_deref());
    WebUrl::try_from(url.as_str()).unwrap_or_else(|_| server.clone())
}

/// `url` opening `folder`, which `code serve-web` takes as its `folder`
/// query parameter, in place of any it had. The rest of the query stays
/// exactly as written, so the token is passed on unchanged.
pub(super) fn with_folder(url: &WebUrl, folder: &str) -> WebUrl {
    let is_folder = |pair: &&str| is_parameter(pair, FOLDER);
    let param = url::form_urlencoded::Serializer::new(String::new())
        .append_pair(FOLDER, folder)
        .finish();
    let query: Vec<&str> = url
        .0
        .query()
        .unwrap_or_default()
        .split('&')
        .filter(|pair| !pair.is_empty() && !is_folder(pair))
        .chain([param.as_str()])
        .collect();
    let mut opened = url.0.clone();
    opened.set_query(Some(&query.join("&")));
    WebUrl::try_from(opened.as_str()).unwrap_or_else(|_| url.clone())
}

/// Remove connection credentials before any tab state is saved or exported.
pub(in crate::browser) fn without_token(page: &WebUrl) -> WebUrl {
    on_server(page, page)
}

fn is_parameter(pair: &str, name: &str) -> bool {
    url::form_urlencoded::parse(pair.as_bytes())
        .next()
        .is_some_and(|(key, _)| key == name)
}
