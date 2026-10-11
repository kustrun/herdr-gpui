#![allow(clippy::unwrap_used)]
use super::urls::with_token;
use super::*;
use core::prelude::v1::test;
mod ownership;

fn url(text: &str) -> WebUrl {
    WebUrl::try_from(text).unwrap()
}

#[test]
fn a_page_loads_where_it_was_with_the_configured_token() {
    let configured = url("http://127.0.0.1:8000/?tkn=new");
    // The server redirected the token away; the folder stays.
    assert_eq!(
        with_token(&url("http://127.0.0.1:8000/?folder=/x"), &configured),
        url("http://127.0.0.1:8000/?folder=/x&tkn=new")
    );
    // An old token is replaced, not repeated.
    assert_eq!(
        with_token(
            &url("http://127.0.0.1:8000/?tkn=old&folder=/x"),
            &configured
        ),
        url("http://127.0.0.1:8000/?folder=/x&tkn=new")
    );
    assert_eq!(
        with_token(&url("http://127.0.0.1:8000/"), &configured),
        url("http://127.0.0.1:8000/?tkn=new")
    );
}

#[test]
fn a_page_without_a_configured_token_or_on_another_server_is_left_alone() {
    let page = url("http://127.0.0.1:8000/?folder=/x");
    assert_eq!(with_token(&page, &url("http://127.0.0.1:8000/")), page);
    assert_eq!(
        with_token(&page, &url("http://127.0.0.1:9000/?tkn=new")),
        page
    );
}

#[test]
fn a_new_page_opens_the_folder_keeping_the_token() {
    assert_eq!(
        with_folder(&url("http://127.0.0.1:8000/?tkn=x"), "/projects/example"),
        url("http://127.0.0.1:8000/?tkn=x&folder=%2Fprojects%2Fexample")
    );
    // A folder already named is replaced, not repeated.
    assert_eq!(
        with_folder(&url("http://127.0.0.1:8000/?folder=/old&tkn=x"), "/new"),
        url("http://127.0.0.1:8000/?tkn=x&folder=%2Fnew")
    );
    assert_eq!(
        with_folder(&url("http://127.0.0.1:8000/"), "/new"),
        url("http://127.0.0.1:8000/?folder=%2Fnew")
    );
}

#[test]
fn encoded_and_duplicate_credentials_never_enter_saved_navigation() {
    let page = url(
        "http://127.0.0.1:8000/workbench?folder=%2Fprojects%2Fexample&%74kn=fictional-secret&t%6bn=fictional-secret&tkn=fictional-secret&theme=dark#file",
    );
    let clean = url("http://127.0.0.1:8000/workbench?folder=%2Fprojects%2Fexample&theme=dark#file");
    assert_eq!(without_token(&page), clean);
    assert_eq!(
        without_token(&url(
            "http://127.0.0.1:8000/?theme=%64ark&&%74kn=fictional-secret&flag#file"
        )),
        url("http://127.0.0.1:8000/?theme=%64ark&&flag#file")
    );
    assert_eq!(
        with_token(&page, &url("http://127.0.0.1:8000/?tkn=fictional-new")),
        url(
            "http://127.0.0.1:8000/workbench?folder=%2Fprojects%2Fexample&theme=dark&tkn=fictional-new#file"
        )
    );
}

#[test]
fn only_absolute_machine_folders_are_opened() {
    assert!(absolute_folder(&Target::Local, "/projects/example"));
    assert!(!absolute_folder(&Target::Local, "projects/example"));
    assert!(!absolute_folder(&Target::Local, "/projects/example\n"));
    let target = Target::Ssh {
        id: "server-a".into(),
        fingerprint: "a".repeat(64),
    };
    assert!(absolute_folder(&target, "/projects/example"));
    assert!(!absolute_folder(&target, "C:\\projects\\example"));
}

#[test]
fn retarget_keeps_raw_navigation_and_fragment_without_old_credentials() {
    assert_eq!(
        on_server(
            &url(
                "http://127.0.0.1:8000/workbench?folder=%2Fprojects%2Fexample&theme=dark+mode&%74kn=fictional-secret#file"
            ),
            &url("http://127.0.0.1:9000/?tkn=fictional-new#other")
        ),
        url("http://127.0.0.1:9000/workbench?folder=%2Fprojects%2Fexample&theme=dark+mode#file")
    );
}
