use super::*;

fn list(text: &str) -> Vec<String> {
    parse(text)
}

#[test]
fn a_listed_host_and_its_subdomains_are_denied_but_not_a_lookalike() {
    let deny = list("x.com\n");
    assert!(check("https://x.com/home", &deny).is_some());
    assert!(check("https://mobile.x.com/post/1", &deny).is_some());
    assert!(check("https://X.COM./", &deny).is_some());
    // A host that merely ends in the same letters is another site.
    assert_eq!(check("https://notx.com/", &deny), None);
    assert_eq!(check("https://example.com/x.com", &deny), None);
}

#[test]
fn comments_blank_lines_and_case_in_the_file_are_ignored() {
    let deny = parse("# X and its kin\n\n  X.com   # the account keys\nMail.Google.com\n");
    assert_eq!(deny, vec!["x.com".to_string(), "mail.google.com".to_string()]);
    assert!(check("https://mail.google.com/mail", &deny).is_some());
}

#[test]
fn an_unparseable_url_is_refused_when_a_list_is_set() {
    assert!(check("not a url", &list("x.com\n")).is_some());
}

#[test]
fn no_list_set_denies_nothing() {
    assert_eq!(refusal_for(None, "https://x.com/"), None);
}

#[test]
fn a_list_that_cannot_be_read_refuses_every_page() {
    let missing = std::env::temp_dir().join("openhuman-never-open-does-not-exist.txt");
    let reason = refusal_for(Some(missing.as_os_str()), "https://example.com/").expect("fails closed");
    assert!(reason.contains("could not be read"), "{reason}");
}

#[test]
fn the_file_named_by_the_environment_is_the_list() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("browser-deny.txt");
    std::fs::write(&file, "never-open.example\n").unwrap();
    assert!(refusal_for(Some(file.as_os_str()), "https://www.never-open.example/a").is_some());
    assert_eq!(refusal_for(Some(file.as_os_str()), "https://example.com/"), None);
}
