use super::*;

#[test]
fn extracts_username_from_canonical_url() {
    let text = "Check out https://www.linkedin.com/in/williamhgates for more";
    let caps = LINKEDIN_USERNAME_RE.captures(text).unwrap();
    assert_eq!(&caps[1], "williamhgates");
    assert_eq!(
        canonical_linkedin_url(&caps[1]),
        "https://www.linkedin.com/in/williamhgates"
    );
}

#[test]
fn extracts_username_from_comm_url() {
    let text = "https://www.linkedin.com/comm/in/stevenenamakel?midToken=abc";
    let caps = LINKEDIN_USERNAME_RE.captures(text).unwrap();
    assert_eq!(&caps[1], "stevenenamakel");
    assert_eq!(
        canonical_linkedin_url(&caps[1]),
        "https://www.linkedin.com/in/stevenenamakel"
    );
}

#[test]
fn extracts_username_from_http_variant() {
    let text = "See http://www.linkedin.com/in/jeannie-wyrick-b4760710a";
    let caps = LINKEDIN_USERNAME_RE.captures(text).unwrap();
    assert_eq!(&caps[1], "jeannie-wyrick-b4760710a");
}

#[test]
fn skips_non_profile_linkedin_urls() {
    let text = "Visit https://www.linkedin.com/company/openai";
    assert!(LINKEDIN_USERNAME_RE.captures(text).is_none());
}

#[test]
fn handles_no_match() {
    assert!(LINKEDIN_USERNAME_RE.captures("No LinkedIn here").is_none());
}
