use super::*;

#[test]
fn renders_headline_location_experience_and_education() {
    let data = json!({
        "fullName": "Ada Lovelace",
        "headline": "Analyst",
        "addressWithCountry": "London, UK",
        "about": "First programmer.",
        "connections": 10,
        "followers": 20,
        "experiences": [{"title": "Engineer", "subtitle": "Analytical Engine Co", "duration": "2y", "caption": "remote", "description": "Wrote notes"}],
        "educations": [{"title": "Home", "subtitle": "Mathematics"}],
        "languages": [{"name": "English"}, {"name": "French"}],
        "volunteering": [{"title": "Tutor", "subtitle": "Society"}]
    });
    let md = render_profile_markdown("https://www.linkedin.com/in/ada", &data);
    assert!(md.starts_with("# User Profile — Ada Lovelace"));
    assert!(md.contains("**Analyst**"));
    assert!(md.contains("Location: London, UK"));
    assert!(md.contains("Connections: 10 | Followers: 20"));
    assert!(md.contains("## About\n\nFirst programmer."));
    assert!(md.contains("- **Engineer** at Analytical Engine Co (2y) — remote"));
    assert!(md.contains("  Wrote notes"));
    assert!(md.contains("- **Home** — Mathematics"));
    assert!(md.contains("Languages: English, French"));
    assert!(md.contains("- Tutor at Society"));
}

#[test]
fn renders_only_the_url_for_an_empty_profile() {
    let md = render_profile_markdown("https://www.linkedin.com/in/x", &json!({}));
    assert!(md.contains("LinkedIn: https://www.linkedin.com/in/x"));
    assert!(!md.contains("## Experience"));
    assert!(!md.contains("## About"));
}
